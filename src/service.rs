extern crate bigdecimal;

use beancounter_grpc::proto;
use beancounter_grpc::proto::*;
use beancounter_grpc::tower_grpc::{Code, Request, Response, Status};
use futures::future::FutureResult;
use instrumented::{instrument, prometheus, register};

use crate::models;
use crate::schema;
use crate::stripe_client;

// This amount is calculated by subtracting Stripe's maximum fee of 2.9% + 30c
// from their charge maximum, which is $999,999.99 according to
// https://stripe.com/docs/currencies#minimum-and-maximum-charge-amounts.
// Thus, it's calculated like so (w/ Python):
//   >>> 99999999 - (99999999 * 0.029 + 30)
//   97099969.0292
static MAX_PAYMENT_AMOUNT: i32 = 97_099_969;

lazy_static! {
    static ref PAYMENT_ADDED: prometheus::HistogramVec = {
        let histogram_opts = prometheus::HistogramOpts::new(
            "payment_added_amount",
            "Histogram of payment added amounts",
        );
        let histogram = prometheus::HistogramVec::new(histogram_opts, &[]).unwrap();

        register(Box::new(histogram.clone())).unwrap();

        histogram
    };
    static ref PAYMENT_ADDED_FEE: prometheus::HistogramVec = {
        let histogram_opts = prometheus::HistogramOpts::new(
            "payment_added_fee_amount",
            "Histogram of payment added fee amounts",
        );
        let histogram = prometheus::HistogramVec::new(histogram_opts, &[]).unwrap();

        register(Box::new(histogram.clone())).unwrap();

        histogram
    };
    static ref PAYMENT_SETTLED: prometheus::HistogramVec = {
        let histogram_opts = prometheus::HistogramOpts::new(
            "payment_settled_amount",
            "Histogram of payment settled amounts",
        );
        let histogram = prometheus::HistogramVec::new(histogram_opts, &[]).unwrap();

        register(Box::new(histogram.clone())).unwrap();

        histogram
    };
    static ref PAYMENT_SETTLED_FEE: prometheus::HistogramVec = {
        let histogram_opts = prometheus::HistogramOpts::new(
            "payment_settled_fee_amount",
            "Histogram of payment settled fee amounts",
        );
        let histogram = prometheus::HistogramVec::new(histogram_opts, &[]).unwrap();

        register(Box::new(histogram.clone())).unwrap();

        histogram
    };
}

#[derive(Clone)]
pub struct BeanCounter {
    db_reader: diesel::r2d2::Pool<diesel::r2d2::ConnectionManager<diesel::pg::PgConnection>>,
    db_writer: diesel::r2d2::Pool<diesel::r2d2::ConnectionManager<diesel::pg::PgConnection>>,
}

#[derive(Debug, Fail)]
enum RequestError {
    #[fail(display = "not found")]
    NotFound,
    #[fail(display = "database error: {}", err)]
    DatabaseError { err: String },
    #[fail(display = "invalid client_id: {}", err)]
    InvalidClientId { err: String },
    #[fail(display = "Bad arguments specified for request")]
    BadArguments,
    #[fail(display = "stripe error: {}", err)]
    StripeError { err: String },
}

impl From<stripe_client::StripeError> for RequestError {
    fn from(err: stripe_client::StripeError) -> Self {
        Self::StripeError {
            err: err.to_string(),
        }
    }
}

impl From<diesel::result::Error> for RequestError {
    fn from(err: diesel::result::Error) -> RequestError {
        match err {
            diesel::result::Error::NotFound => RequestError::NotFound,
            _ => RequestError::DatabaseError {
                err: format!("{}", err),
            },
        }
    }
}

impl From<uuid::parser::ParseError> for RequestError {
    fn from(err: uuid::parser::ParseError) -> RequestError {
        RequestError::InvalidClientId {
            err: format!("{}", err),
        }
    }
}

impl From<&models::Transaction> for Transaction {
    fn from(tx: &models::Transaction) -> Self {
        use crate::sql_types::TransactionType;
        Self {
            client_id: tx.client_id.unwrap().to_simple().to_string(),
            created_at: Some(tx.created_at.into()),
            amount_cents: tx.amount_cents,
            tx_type: match tx.tx_type {
                TransactionType::Credit => transaction::Type::Credit,
                TransactionType::PromoCredit => transaction::Type::PromoCredit,
                TransactionType::Debit => transaction::Type::Debit,
            } as i32,
        }
    }
}

impl From<models::Balance> for beancounter_grpc::proto::Balance {
    fn from(balance: models::Balance) -> Self {
        Self {
            client_id: balance.client_id.to_simple().to_string(),
            balance_cents: balance.balance_cents,
            promo_cents: balance.promo_cents,
        }
    }
}

fn calculate_balance(credit_sum: i64, promo_credit_sum: i64, debit_sum: i64) -> (i64, i64) {
    // Debits are negative, and credits are positive. Thus, adding a debit to a
    // credit is equivalent to subtraction.

    // Add debits to promo balance first
    let mut promo_cents_remaining = promo_credit_sum + debit_sum;
    let debit_remaining = promo_cents_remaining;
    if promo_cents_remaining < 0 {
        // The promo balance should never be negative
        promo_cents_remaining = 0;
    }

    // Add any remaining debits to the final balance
    let balance_cents_remaining = if debit_remaining < 0 {
        credit_sum + debit_remaining
    } else {
        credit_sum
    };

    (balance_cents_remaining, promo_cents_remaining)
}

#[instrument(INFO)]
fn update_and_return_balance(
    client_uuid: uuid::Uuid,
    conn: &diesel::r2d2::PooledConnection<diesel::r2d2::ConnectionManager<diesel::PgConnection>>,
) -> Result<models::Balance, diesel::result::Error> {
    use crate::models::*;
    use crate::sql_types::*;
    use diesel::dsl::*;
    use diesel::insert_into;
    use diesel::prelude::*;
    use schema::balances::table as balances;
    use schema::transactions::columns::*;
    use schema::transactions::table as transactions;

    let credit_sum = transactions
        .filter(
            tx_type
                .eq(TransactionType::Credit)
                .and(client_id.eq(client_uuid)),
        )
        .select(sum(amount_cents))
        .first::<Option<i64>>(conn)?
        .unwrap_or_else(|| 0);

    let promo_credit_sum = transactions
        .filter(
            tx_type
                .eq(TransactionType::PromoCredit)
                .and(client_id.eq(client_uuid)),
        )
        .select(sum(amount_cents))
        .first::<Option<i64>>(conn)?
        .unwrap_or_else(|| 0);

    let debit_sum = transactions
        .filter(
            tx_type
                .eq(TransactionType::Debit)
                .and(client_id.eq(client_uuid)),
        )
        .select(sum(amount_cents))
        .first::<Option<i64>>(conn)?
        .unwrap_or_else(|| 0);

    let (balance_cents_remaining, promo_cents_remaining) =
        calculate_balance(credit_sum, promo_credit_sum, debit_sum);

    Ok(insert_into(balances)
        .values(&NewBalance {
            client_id: client_uuid,
            balance_cents: balance_cents_remaining,
            promo_cents: promo_cents_remaining,
        })
        .on_conflict(schema::balances::columns::client_id)
        .do_update()
        .set(&UpdatedBalance {
            balance_cents: balance_cents_remaining,
            promo_cents: 0,
        })
        .get_result(conn)?)
}

#[instrument(INFO)]
pub fn add_transaction(
    client_id_credit: Option<uuid::Uuid>,
    client_id_debit: Option<uuid::Uuid>,
    amount_cents: i32,
    conn: &diesel::r2d2::PooledConnection<diesel::r2d2::ConnectionManager<diesel::PgConnection>>,
) -> Result<(models::Transaction, models::Transaction), diesel::result::Error> {
    use crate::models::*;
    use crate::sql_types::*;
    use diesel::prelude::*;
    use schema::transactions::table as transactions;

    let tx_credit = NewTransaction {
        client_id: client_id_credit,
        tx_type: TransactionType::Credit,
        amount_cents,
    };
    let tx_debit = NewTransaction {
        client_id: client_id_debit,
        tx_type: TransactionType::Debit,
        amount_cents: -amount_cents, // Debits should be negative
    };

    let tx_credit = diesel::insert_into(transactions)
        .values(&tx_credit)
        .get_result::<Transaction>(conn)?;

    let tx_debit = diesel::insert_into(transactions)
        .values(&tx_debit)
        .get_result::<Transaction>(conn)?;

    Ok((tx_credit, tx_debit))
}

impl BeanCounter {
    pub fn new(
        db_reader: diesel::r2d2::Pool<diesel::r2d2::ConnectionManager<diesel::pg::PgConnection>>,
        db_writer: diesel::r2d2::Pool<diesel::r2d2::ConnectionManager<diesel::pg::PgConnection>>,
    ) -> Self {
        BeanCounter {
            db_reader,
            db_writer,
        }
    }

    #[instrument(INFO)]
    fn handle_get_balance(
        &self,
        request: &GetBalanceRequest,
    ) -> Result<GetBalanceResponse, RequestError> {
        use crate::models::*;
        use diesel::prelude::*;
        use diesel::result::Error;
        use uuid::Uuid;

        let client_uuid = Uuid::parse_str(&request.client_id)?;

        let balance = self.get_balance(client_uuid)?;

        Ok(GetBalanceResponse {
            balance: Some(beancounter_grpc::proto::Balance {
                client_id: balance.client_id.to_simple().to_string(),
                balance_cents: balance.balance_cents,
                promo_cents: balance.promo_cents,
            }),
        })
    }

    #[instrument(INFO)]
    fn get_balance(
        &self,
        client_uuid: uuid::Uuid,
    ) -> Result<models::Balance, diesel::result::Error> {
        use crate::models::*;
        use crate::schema::balances::columns::*;
        use crate::schema::balances::table as balances;
        use diesel::insert_into;
        use diesel::prelude::*;

        let reader_conn = self.db_reader.get().unwrap();
        let result = balances
            .filter(client_id.eq(client_uuid))
            .first(&reader_conn);

        match result {
            // If the balance record exists, return that
            Ok(result) => Ok(result),
            // If there's no record yet, create a new zeroed out balance record.
            Err(diesel::NotFound) => {
                let writer_conn = self.db_reader.get().unwrap();
                Ok(insert_into(balances)
                    .values(&NewZeroBalance {
                        client_id: client_uuid,
                    })
                    .get_result(&writer_conn)?)
            }
            Err(err) => Err(err),
        }
    }

    #[instrument(INFO)]
    fn handle_get_transactions(
        &self,
        request: &GetTransactionsRequest,
    ) -> Result<GetTransactionsResponse, RequestError> {
        use diesel::prelude::*;
        use diesel::result::Error;
        use schema::transactions::columns::*;
        use schema::transactions::table as transactions;
        use uuid::Uuid;

        let client_uuid = Uuid::parse_str(&request.client_id)?;

        let conn = self.db_reader.get().unwrap();
        let tx_vec =
            conn.transaction::<Vec<beancounter_grpc::proto::Transaction>, Error, _>(|| {
                let result = transactions
                    .filter(client_id.eq(client_uuid))
                    .get_results(&conn)?;

                Ok(result
                    .iter()
                    .map(beancounter_grpc::proto::Transaction::from)
                    .collect())
            })?;

        Ok(GetTransactionsResponse {
            transactions: tx_vec,
        })
    }

    #[instrument(INFO)]
    fn handle_add_credits(
        &self,
        request: &AddCreditsRequest,
    ) -> Result<AddCreditsResponse, RequestError> {
        use crate::models::*;
        use diesel::prelude::*;
        use diesel::result::Error;
        use uuid::Uuid;

        let client_uuid = Uuid::parse_str(&request.client_id)?;

        let conn = self.db_writer.get().unwrap();
        let balance = conn.transaction::<Balance, Error, _>(|| {
            add_transaction(Some(client_uuid), None, request.amount_cents, &conn)?;

            Ok(update_and_return_balance(client_uuid, &conn)?)
        })?;

        Ok(AddCreditsResponse {
            balance: Some(balance.into()),
        })
    }

    #[instrument(INFO)]
    fn handle_withdraw_credits(
        &self,
        request: &WithdrawCreditsRequest,
    ) -> Result<WithdrawCreditsResponse, RequestError> {
        Err(RequestError::BadArguments)
    }

    #[instrument(INFO)]
    fn handle_add_payment(
        &self,
        request: &AddPaymentRequest,
    ) -> Result<AddPaymentResponse, RequestError> {
        use crate::models::NewPayment;
        use crate::models::*;
        use data_encoding::BASE64_NOPAD;
        use diesel::insert_into;
        use diesel::prelude::*;
        use diesel::result::Error;
        use schema::payments::table as payments;
        use uuid::Uuid;

        let client_uuid_from = Uuid::parse_str(&request.client_id_from)?;
        let client_uuid_to = Uuid::parse_str(&request.client_id_to)?;

        let payment_cents = request.payment_cents;
        let fee_cents = (f64::from(payment_cents) * 0.15).floor() as i32;
        let total_amount = payment_cents + fee_cents;

        // Any payment over this amount will never go through
        if total_amount >= MAX_PAYMENT_AMOUNT {
            return Ok(AddPaymentResponse {
                result: add_payment_response::Result::InvalidAmount as i32,
                payment_cents: 0,
                fee_cents: 0,
                balance: None,
            });
        }

        let conn = self.db_writer.get().unwrap();
        // Check the sender balance, make sure it's sufficient.
        let balance = self.get_balance(client_uuid_from)?;
        if balance.balance_cents + balance.promo_cents < i64::from(total_amount) {
            return Ok(AddPaymentResponse {
                result: add_payment_response::Result::InsufficientBalance as i32,
                payment_cents: 0,
                fee_cents: 0,
                balance: Some(balance.into()),
            });
        }

        let balance = conn.transaction::<Balance, Error, _>(|| {
            // Zero value payments are perfectly valid; they simply don't generate
            // a TX
            if total_amount > 0 {
                // Credit the cash account, debit the sender. This TX is
                // refundable.
                add_transaction(None, Some(client_uuid_from), payment_cents, &conn)?;

                // Credit the cash account, debit the sender. This TX is non-refundable.
                add_transaction(None, Some(client_uuid_from), fee_cents, &conn)?;
            }

            // Finally, create a payment record.
            let payment = NewPayment {
                client_id_from: client_uuid_from,
                client_id_to: client_uuid_to,
                payment_cents,
                message_hash: BASE64_NOPAD.encode(&request.message_hash),
            };
            insert_into(payments).values(&payment).execute(&conn)?;

            Ok(update_and_return_balance(client_uuid_from, &conn)?)
        })?;

        PAYMENT_ADDED
            .with_label_values(&[])
            .observe(f64::from(payment_cents));
        PAYMENT_ADDED_FEE
            .with_label_values(&[])
            .observe(f64::from(fee_cents));

        Ok(AddPaymentResponse {
            result: add_payment_response::Result::Success as i32,
            payment_cents,
            fee_cents,
            balance: Some(balance.into()),
        })
    }

    #[instrument(INFO)]
    fn handle_settle_payment(
        &self,
        request: &SettlePaymentRequest,
    ) -> Result<SettlePaymentResponse, RequestError> {
        use crate::models::*;
        use crate::schema::payments::columns::*;
        use crate::schema::payments::table as payments;
        use data_encoding::BASE64_NOPAD;
        use diesel::prelude::*;
        use diesel::result::Error;

        let conn = self.db_writer.get().unwrap();
        let (payment_amount_after_fee, fee_amount, balance) = conn
            .transaction::<(i32, i32, Balance), Error, _>(|| {
                let payment: Payment = payments
                    .filter(message_hash.eq(BASE64_NOPAD.encode(&request.message_hash)))
                    .first(&conn)?;

                // If there's a valid payment, perform settlement
                let fee_amount = (f64::from(payment.payment_cents) * 0.15).floor() as i32;
                let payment_amount_after_fee = payment.payment_cents - fee_amount;

                // Add TX from umpyre cash account to recipient
                add_transaction(
                    Some(payment.client_id_to),
                    None,
                    payment_amount_after_fee,
                    &conn,
                )?;

                // delete the payment
                diesel::delete(payments)
                    .filter(message_hash.eq(BASE64_NOPAD.encode(&request.message_hash)))
                    .execute(&conn)?;

                let balance = update_and_return_balance(payment.client_id_to, &conn)?;

                Ok((payment_amount_after_fee, fee_amount, balance))
            })?;

        PAYMENT_SETTLED
            .with_label_values(&[])
            .observe(f64::from(payment_amount_after_fee));
        PAYMENT_SETTLED_FEE
            .with_label_values(&[])
            .observe(f64::from(fee_amount));

        Ok(SettlePaymentResponse {
            fee_cents: fee_amount,
            payment_cents: payment_amount_after_fee,
            balance: Some(balance.into()),
        })
    }

    #[instrument(INFO)]
    fn handle_stripe_charge(
        &self,
        request: &StripeChargeRequest,
    ) -> Result<StripeChargeResponse, RequestError> {
        use crate::stripe_client::{Stripe, StripeError};
        use diesel::prelude::*;
        use diesel::result::Error;
        use uuid::Uuid;

        let client_uuid = Uuid::parse_str(&request.client_id)?;
        let mut charge_response: Option<StripeChargeResponse> = None;

        let conn = self.db_writer.get().unwrap();
        let _db_result = conn.transaction::<_, Error, _>(|| {
            let stripe_fee_amount_cents =
                Stripe::calculate_stripe_fees(i64::from(request.amount_cents));

            // Add TX from cash account to client, minus fees
            let (tx_credit, _tx_debit) = add_transaction(
                Some(client_uuid),
                None,
                (i64::from(request.amount_cents) - stripe_fee_amount_cents) as i32,
                &conn,
            )?;

            let stripe = Stripe::new();

            let charge_result = stripe.charge(
                &request.token,
                i64::from(request.amount_cents),
                &request.client_id,
                tx_credit.id,
            );

            match charge_result {
                Ok(charge) => {
                    let balance = update_and_return_balance(client_uuid, &conn)?;
                    charge_response = Some(StripeChargeResponse {
                        result: stripe_charge_response::Result::Success as i32,
                        api_response: serde_json::to_string(&charge).unwrap(),
                        message: "".into(),
                        balance: Some(balance.into()),
                    });
                    Ok(())
                }
                Err(StripeError::RequestError { request_error, .. }) => {
                    charge_response = Some(StripeChargeResponse {
                        result: stripe_charge_response::Result::Failure as i32,
                        api_response: serde_json::to_string(&request_error).unwrap(),
                        message: "".into(),
                        balance: None,
                    });
                    Err(Error::RollbackTransaction)
                }
                Err(err) => {
                    charge_response = Some(StripeChargeResponse {
                        result: stripe_charge_response::Result::Failure as i32,
                        api_response: "".into(),
                        message: err.to_string(),
                        balance: None,
                    });
                    Err(Error::RollbackTransaction)
                }
            }
        });

        match charge_response {
            Some(response) => Ok(response),
            None => Err(RequestError::BadArguments),
        }
    }
}

impl proto::server::BeanCounter for BeanCounter {
    type GetBalanceFuture = FutureResult<Response<GetBalanceResponse>, Status>;
    type GetTransactionsFuture = FutureResult<Response<GetTransactionsResponse>, Status>;
    type AddCreditsFuture = FutureResult<Response<AddCreditsResponse>, Status>;
    type WithdrawCreditsFuture = FutureResult<Response<WithdrawCreditsResponse>, Status>;
    type AddPaymentFuture = FutureResult<Response<AddPaymentResponse>, Status>;
    type SettlePaymentFuture = FutureResult<Response<SettlePaymentResponse>, Status>;
    type StripeChargeFuture = FutureResult<Response<StripeChargeResponse>, Status>;
    type CheckFuture = FutureResult<Response<HealthCheckResponse>, Status>;

    /// Get account balance
    fn get_balance(&mut self, request: Request<GetBalanceRequest>) -> Self::GetBalanceFuture {
        use futures::future::IntoFuture;
        self.handle_get_balance(request.get_ref())
            .map(Response::new)
            .map_err(|err| Status::new(Code::InvalidArgument, err.to_string()))
            .into_future()
    }

    /// Get transactions
    fn get_transactions(
        &mut self,
        request: Request<GetTransactionsRequest>,
    ) -> Self::GetTransactionsFuture {
        use futures::future::IntoFuture;
        self.handle_get_transactions(request.get_ref())
            .map(Response::new)
            .map_err(|err| Status::new(Code::InvalidArgument, err.to_string()))
            .into_future()
    }

    /// Add credits
    fn add_credits(&mut self, request: Request<AddCreditsRequest>) -> Self::AddCreditsFuture {
        use futures::future::IntoFuture;
        self.handle_add_credits(request.get_ref())
            .map(Response::new)
            .map_err(|err| Status::new(Code::InvalidArgument, err.to_string()))
            .into_future()
    }

    /// Withdraw credits
    fn withdraw_credits(
        &mut self,
        request: Request<WithdrawCreditsRequest>,
    ) -> Self::WithdrawCreditsFuture {
        use futures::future::IntoFuture;
        self.handle_withdraw_credits(request.get_ref())
            .map(Response::new)
            .map_err(|err| Status::new(Code::InvalidArgument, err.to_string()))
            .into_future()
    }

    /// Add a payment
    fn add_payment(&mut self, request: Request<AddPaymentRequest>) -> Self::AddPaymentFuture {
        use futures::future::IntoFuture;
        self.handle_add_payment(request.get_ref())
            .map(Response::new)
            .map_err(|err| Status::new(Code::InvalidArgument, err.to_string()))
            .into_future()
    }

    /// Settle a payment
    fn settle_payment(
        &mut self,
        request: Request<SettlePaymentRequest>,
    ) -> Self::SettlePaymentFuture {
        use futures::future::IntoFuture;
        self.handle_settle_payment(request.get_ref())
            .map(Response::new)
            .map_err(|err| Status::new(Code::InvalidArgument, err.to_string()))
            .into_future()
    }

    /// Create a stripe charge
    fn stripe_charge(&mut self, request: Request<StripeChargeRequest>) -> Self::StripeChargeFuture {
        use futures::future::IntoFuture;
        self.handle_stripe_charge(request.get_ref())
            .map(Response::new)
            .map_err(|err| Status::new(Code::InvalidArgument, err.to_string()))
            .into_future()
    }

    /// Health check endpoint
    fn check(&mut self, _request: Request<HealthCheckRequest>) -> Self::CheckFuture {
        use futures::future::ok;
        ok(Response::new(HealthCheckResponse {
            status: proto::health_check_response::ServingStatus::Serving as i32,
        }))
    }
}

#[cfg(test)]
mod tests {
    extern crate rand;

    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;
    use diesel::dsl::*;
    use diesel::pg::PgConnection;
    use diesel::prelude::*;
    use diesel::r2d2::{ConnectionManager, Pool};
    use std::sync::Mutex;
    use uuid::Uuid;

    lazy_static! {
        static ref LOCK: Mutex<i32> = Mutex::new(0);
    }

    fn get_pools(
    ) -> (diesel::r2d2::Pool<diesel::r2d2::ConnectionManager<diesel::pg::PgConnection>>,) {
        let pg_manager = ConnectionManager::<PgConnection>::new(
            "postgres://postgres:password@127.0.0.1:5432/beancounter",
        );
        let db_pool = Pool::builder().build(pg_manager).unwrap();

        (db_pool,)
    }

    fn empty_tables(
        db_pool: &diesel::r2d2::Pool<diesel::r2d2::ConnectionManager<diesel::pg::PgConnection>>,
    ) {
        let conn = db_pool.get().unwrap();

        macro_rules! empty_tables {
                ( $( $x:ident ),* ) => {
                $(
                    diesel::delete(schema::$x::table).execute(&conn).unwrap();
                    assert_eq!(Ok(0), schema::$x::table.select(count(schema::$x::id)).first(&conn));
                )*
            };
        }

        empty_tables![transactions, balances, payments];
    }

    fn check_zero_sum(
        db_pool: &diesel::r2d2::Pool<diesel::r2d2::ConnectionManager<diesel::pg::PgConnection>>,
    ) {
        let conn = db_pool.get().unwrap();

        // All credits are positive, and all debits are negative. When summed,
        // they should always balance out to 0.
        let tx_sum = schema::transactions::table
            .select(sum(schema::transactions::dsl::amount_cents))
            .first::<Option<i64>>(&conn)
            .unwrap();
        assert_eq!(Some(0), tx_sum);
    }

    #[test]
    fn test_add_credits() {
        use diesel::prelude::*;
        use schema::transactions::columns::*;
        use schema::transactions::table as transactions;

        let _lock = LOCK.lock().unwrap();

        let (db_pool,) = get_pools();

        empty_tables(&db_pool);

        let beancounter = BeanCounter::new(db_pool.clone(), db_pool.clone());

        // generate 100 UUIDs
        let mut uuids = Vec::<String>::new();
        for _ in 0..100 {
            uuids.push(Uuid::new_v4().to_simple().to_string());
        }

        for uuid in uuids.iter() {
            let amount = 100;
            let result = beancounter.handle_add_credits(&AddCreditsRequest {
                client_id: uuid.clone(),
                amount_cents: amount,
            });

            assert!(result.is_ok());
            let balance = result.unwrap().balance.unwrap();
            assert_eq!(balance.balance_cents, i64::from(amount));
            assert_eq!(balance.promo_cents, 0);
        }

        let conn = db_pool.get().unwrap();

        let tx_count = transactions.select(count(id)).first(&conn);
        assert_eq!(Ok(200), tx_count);

        check_zero_sum(&db_pool);

        for uuid in uuids.iter() {
            let balance_result = beancounter.handle_get_balance(&GetBalanceRequest {
                client_id: uuid.clone(),
            });

            assert!(balance_result.is_ok());
            let balance = balance_result.unwrap().balance.unwrap();
            assert_eq!(balance.balance_cents, 100);
            assert_eq!(balance.promo_cents, 0);
        }
    }

    #[test]
    fn test_calculate_balance() {
        let (balance, promo) = calculate_balance(0, 0, 0);
        assert_eq!(balance, 0);
        assert_eq!(promo, 0);

        let (balance, promo) = calculate_balance(10, 0, 0);
        assert_eq!(balance, 10);
        assert_eq!(promo, 0);

        let (balance, promo) = calculate_balance(10, 0, -10);
        assert_eq!(balance, 0);
        assert_eq!(promo, 0);

        let (balance, promo) = calculate_balance(10, 10, -10);
        assert_eq!(balance, 10);
        assert_eq!(promo, 0);

        let (balance, promo) = calculate_balance(10, 10, -20);
        assert_eq!(balance, 0);
        assert_eq!(promo, 0);

        let (balance, promo) = calculate_balance(0, 10, -10);
        assert_eq!(balance, 0);
        assert_eq!(promo, 0);

        // These cases (negative balance) should never occur, but we test for
        // it here anyway, just to make sure the math is right.
        let (balance, promo) = calculate_balance(0, 10, -20);
        assert_eq!(balance, -10);
        assert_eq!(promo, 0);

        let (balance, promo) = calculate_balance(10, 0, -20);
        assert_eq!(balance, -10);
        assert_eq!(promo, 0);
    }

    #[test]
    fn test_get_balance() {
        use rand::Rng;

        let _lock = LOCK.lock().unwrap();

        let (db_pool,) = get_pools();

        empty_tables(&db_pool);

        let beancounter = BeanCounter::new(db_pool.clone(), db_pool.clone());

        // A fresh new client_id returns a zero balance.
        let balance_result = beancounter.handle_get_balance(&GetBalanceRequest {
            client_id: Uuid::new_v4().to_simple().to_string(),
        });

        assert!(balance_result.is_ok());
        let balance = balance_result.unwrap().balance.unwrap();
        assert_eq!(balance.balance_cents, 0);
        assert_eq!(balance.promo_cents, 0);

        // Add some credits to a new client, check the balance
        let mut rng = rand::thread_rng();
        let uuid = Uuid::new_v4().to_simple().to_string();
        let amount = rng.gen_range(0, 999_999_999);
        let result = beancounter.handle_add_credits(&AddCreditsRequest {
            client_id: uuid.clone(),
            amount_cents: amount,
        });

        assert!(result.is_ok());
        let balance = result.unwrap().balance.unwrap();
        assert_eq!(balance.balance_cents, i64::from(amount));
        assert_eq!(balance.promo_cents, 0);

        let balance_result = beancounter.handle_get_balance(&GetBalanceRequest { client_id: uuid });

        assert!(balance_result.is_ok());
        let balance = balance_result.unwrap().balance.unwrap();
        assert_eq!(balance.balance_cents, i64::from(amount));
        assert_eq!(balance.promo_cents, 0);
        check_zero_sum(&db_pool);
    }

    #[test]
    fn test_get_transactions() {
        use crate::sql_types::TransactionType;
        use rand::Rng;

        let _lock = LOCK.lock().unwrap();

        let (db_pool,) = get_pools();

        empty_tables(&db_pool);

        let beancounter = BeanCounter::new(db_pool.clone(), db_pool.clone());

        let uuid = Uuid::new_v4().to_simple().to_string();

        // Brand new client, no transactions (yet)
        let tx_result = beancounter.handle_get_transactions(&GetTransactionsRequest {
            client_id: uuid.clone(),
        });

        assert!(tx_result.is_ok());
        let tx_result = tx_result.unwrap();
        assert!(tx_result.transactions.is_empty());

        // Add some credits to a new client, check the balance
        let mut rng = rand::thread_rng();
        let uuid = Uuid::new_v4().to_simple().to_string();
        let amount = rng.gen_range(0, 999_999_999);
        let result = beancounter.handle_add_credits(&AddCreditsRequest {
            client_id: uuid.clone(),
            amount_cents: amount,
        });

        assert!(result.is_ok());
        let balance = result.unwrap().balance.unwrap();
        assert_eq!(balance.balance_cents, i64::from(amount));
        assert_eq!(balance.promo_cents, 0);

        // There should be some TXs present now
        let tx_result = beancounter.handle_get_transactions(&GetTransactionsRequest {
            client_id: uuid.clone(),
        });

        assert!(tx_result.is_ok());
        let tx_result = tx_result.unwrap();
        assert!(!tx_result.transactions.is_empty());
        assert_eq!(tx_result.transactions.len(), 1);
        assert_eq!(tx_result.transactions[0].amount_cents, amount);
        assert_eq!(
            tx_result.transactions[0].tx_type,
            transaction::Type::Credit as i32
        );

        let conn = db_pool.get().unwrap();

        // Check there's a corresponding debit against the umpyre cash account
        let result: Vec<models::Transaction> = schema::transactions::table
            .filter(schema::transactions::dsl::client_id.is_null())
            .get_results(&conn)
            .unwrap();

        assert!(!result.is_empty());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].amount_cents, -amount);
        assert_eq!(result[0].tx_type, TransactionType::Debit);

        check_zero_sum(&db_pool);
    }

    #[test]
    fn test_add_payment() {
        use rand::RngCore;

        let _lock = LOCK.lock().unwrap();

        let (db_pool,) = get_pools();

        empty_tables(&db_pool);

        let beancounter = BeanCounter::new(db_pool.clone(), db_pool.clone());

        for payment_amount in 0..50 {
            let client_uuid_from = Uuid::new_v4().to_simple().to_string();
            let client_uuid_to = Uuid::new_v4().to_simple().to_string();
            let mut message_hash = vec![0u8; 32];
            rand::thread_rng().fill_bytes(&mut message_hash);

            if payment_amount > 0 {
                // This should fail due to insufficient balance
                let result = beancounter.handle_add_payment(&AddPaymentRequest {
                    client_id_from: client_uuid_from.clone(),
                    client_id_to: client_uuid_to.clone(),
                    message_hash: message_hash.clone(),
                    payment_cents: payment_amount,
                });

                assert!(result.is_ok());
                let result = result.unwrap();
                assert_eq!(
                    result.result,
                    add_payment_response::Result::InsufficientBalance as i32
                );
                assert_eq!(result.payment_cents, 0);
            }

            // Add some credits to sender's account
            let result = beancounter.handle_add_credits(&AddCreditsRequest {
                client_id: client_uuid_from.clone(),
                amount_cents: payment_amount,
            });

            assert!(result.is_ok());
            let balance = result.unwrap().balance.unwrap();
            assert_eq!(balance.balance_cents, i64::from(payment_amount));
            assert_eq!(balance.promo_cents, 0);

            if payment_amount > 7 {
                // This should still fail due to insufficient balance, because we're not
                // accounting for the fee
                let result = beancounter.handle_add_payment(&AddPaymentRequest {
                    client_id_from: client_uuid_from.clone(),
                    client_id_to: client_uuid_to.clone(),
                    message_hash: message_hash.clone(),
                    payment_cents: payment_amount,
                });

                assert!(result.is_ok());
                let result = result.unwrap();
                assert_eq!(
                    result.result,
                    add_payment_response::Result::InsufficientBalance as i32
                );
                assert_eq!(result.payment_cents, 0);
            }

            // Try again, but reduce the payment so that we can afford the fee
            // This should still fail due to insufficient balance, because we're not
            // accounting for the fee
            let payment_cents = (f64::from(payment_amount) / 1.15).round() as i32;
            let fee_cents = (f64::from(payment_cents) * 0.15).floor() as i32;
            let result = beancounter.handle_add_payment(&AddPaymentRequest {
                client_id_from: client_uuid_from.clone(),
                client_id_to: client_uuid_to.clone(),
                message_hash: message_hash.clone(),
                payment_cents,
            });

            assert!(result.is_ok());
            let result = result.unwrap();
            assert_eq!(result.result, add_payment_response::Result::Success as i32);
            assert_eq!(result.payment_cents, payment_cents);
            assert_eq!(result.fee_cents, fee_cents);

            let conn = db_pool.get().unwrap();

            // Check balance of sender
            let sender_balance = beancounter
                .get_balance(Uuid::parse_str(&client_uuid_from).unwrap())
                .unwrap();
            assert_eq!(
                sender_balance.balance_cents,
                i64::from(payment_amount - (payment_cents + fee_cents))
            );
            assert_eq!(sender_balance.promo_cents, 0);

            // Check balance of recipient--should be zero
            let recipient_balance = beancounter
                .get_balance(Uuid::parse_str(&client_uuid_to).unwrap())
                .unwrap();
            assert_eq!(recipient_balance.balance_cents, 0);
            assert_eq!(recipient_balance.promo_cents, 0);
        }

        check_zero_sum(&db_pool);
    }

    #[test]
    fn test_settle_payment() {
        use rand::RngCore;

        let _lock = LOCK.lock().unwrap();

        let (db_pool,) = get_pools();

        empty_tables(&db_pool);

        let beancounter = BeanCounter::new(db_pool.clone(), db_pool.clone());

        for payment_amount in 0..50 {
            let client_uuid_from = Uuid::new_v4().to_simple().to_string();
            let client_uuid_to = Uuid::new_v4().to_simple().to_string();
            let mut message_hash = vec![0u8; 32];
            rand::thread_rng().fill_bytes(&mut message_hash);

            if payment_amount > 0 {
                // This should fail due to insufficient balance
                let result = beancounter.handle_add_payment(&AddPaymentRequest {
                    client_id_from: client_uuid_from.clone(),
                    client_id_to: client_uuid_to.clone(),
                    message_hash: message_hash.clone(),
                    payment_cents: payment_amount,
                });

                assert!(result.is_ok());
                let result = result.unwrap();
                assert_eq!(
                    result.result,
                    add_payment_response::Result::InsufficientBalance as i32
                );
                assert_eq!(result.payment_cents, 0);
            }

            // Add some credits to sender's account
            let result = beancounter.handle_add_credits(&AddCreditsRequest {
                client_id: client_uuid_from.clone(),
                amount_cents: payment_amount,
            });

            assert!(result.is_ok());
            let balance = result.unwrap().balance.unwrap();
            assert_eq!(balance.balance_cents, i64::from(payment_amount));
            assert_eq!(balance.promo_cents, 0);

            if payment_amount > 7 {
                // This should still fail due to insufficient balance, because we're not
                // accounting for the fee
                let result = beancounter.handle_add_payment(&AddPaymentRequest {
                    client_id_from: client_uuid_from.clone(),
                    client_id_to: client_uuid_to.clone(),
                    message_hash: message_hash.clone(),
                    payment_cents: payment_amount,
                });

                assert!(result.is_ok());
                let result = result.unwrap();
                assert_eq!(
                    result.result,
                    add_payment_response::Result::InsufficientBalance as i32
                );
                assert_eq!(result.payment_cents, 0);
            }

            // Try again, but reduce the payment so that we can afford the fee
            // This should still fail due to insufficient balance, because we're not
            // accounting for the fee
            let payment_cents = (f64::from(payment_amount) / 1.15).round() as i32;
            let fee_cents = (f64::from(payment_cents) * 0.15).floor() as i32;
            let result = beancounter.handle_add_payment(&AddPaymentRequest {
                client_id_from: client_uuid_from.clone(),
                client_id_to: client_uuid_to.clone(),
                message_hash: message_hash.clone(),
                payment_cents,
            });

            assert!(result.is_ok());
            let result = result.unwrap();
            assert_eq!(result.result, add_payment_response::Result::Success as i32);
            assert_eq!(result.payment_cents, payment_cents);
            assert_eq!(result.fee_cents, fee_cents);

            let conn = db_pool.get().unwrap();

            // Check balance of sender
            let sender_balance = beancounter
                .get_balance(Uuid::parse_str(&client_uuid_from).unwrap())
                .unwrap();
            assert_eq!(
                sender_balance.balance_cents,
                i64::from(payment_amount - (payment_cents + fee_cents))
            );
            assert_eq!(sender_balance.promo_cents, 0);

            // Check balance of recipient--should be zero
            let recipient_balance = beancounter
                .get_balance(Uuid::parse_str(&client_uuid_to).unwrap())
                .unwrap();
            assert_eq!(recipient_balance.balance_cents, 0);
            assert_eq!(recipient_balance.promo_cents, 0);

            // Try and settle the payment
            let result = beancounter.handle_settle_payment(&SettlePaymentRequest {
                message_hash: message_hash.clone(),
            });

            assert!(result.is_ok());
            let result = result.unwrap();

            // Check balance of recipient--should equal to the payment minus fee
            let recipient_balance = beancounter
                .get_balance(Uuid::parse_str(&client_uuid_to).unwrap())
                .unwrap();
            assert_eq!(
                recipient_balance.balance_cents,
                i64::from(result.payment_cents)
            );
            assert_eq!(recipient_balance.promo_cents, 0);

            // Attempt to settle the payment again, it should fail
            let result = beancounter.handle_settle_payment(&SettlePaymentRequest {
                message_hash: message_hash.clone(),
            });

            assert!(result.is_err());
        }

        check_zero_sum(&db_pool);
    }

    #[test]
    fn test_stripe_charge() {
        let _lock = LOCK.lock().unwrap();

        let (db_pool,) = get_pools();

        empty_tables(&db_pool);

        let beancounter = BeanCounter::new(db_pool.clone(), db_pool.clone());

        let client_id_uuid = Uuid::new_v4();
        let token = r#"
        {
            "id": "tok_visa",
            "object": "token",
            "card": {
                "id": "card_1EYyYcG27b2IeIO74TusmAci",
                "object": "card",
                "address_city": null,
                "address_country": null,
                "address_line1": null,
                "address_line1_check": null,
                "address_line2": null,
                "address_state": null,
                "address_zip": null,
                "address_zip_check": null,
                "brand": "Visa",
                "country": "US",
                "cvc_check": null,
                "dynamic_last4": null,
                "exp_month": 8,
                "exp_year": 2020,
                "fingerprint": "9vruG6eJZVIM6012",
                "funding": "credit",
                "last4": "4242",
                "metadata": {},
                "name": null,
                "tokenization_method": null
            },
            "client_ip": null,
            "created": 1557594022,
            "livemode": false,
            "type": "card",
            "used": false
        }"#;

        let charge_result = beancounter.handle_stripe_charge(&StripeChargeRequest {
            client_id: client_id_uuid.to_simple().to_string(),
            amount_cents: 1000,
            token: token.to_string(),
        });

        assert!(charge_result.is_ok());
        let charge = charge_result.unwrap();

        assert_eq!(charge.balance.as_ref().unwrap().balance_cents, 941);
        assert_eq!(charge.balance.as_ref().unwrap().promo_cents, 0);

        let charge_result = beancounter.handle_stripe_charge(&StripeChargeRequest {
            client_id: client_id_uuid.to_simple().to_string(),
            amount_cents: 10000,
            token: token.to_string(),
        });

        assert!(charge_result.is_ok());
        let charge = charge_result.unwrap();

        assert_eq!(charge.balance.as_ref().unwrap().balance_cents, 10621);
        assert_eq!(charge.balance.as_ref().unwrap().promo_cents, 0);

        check_zero_sum(&db_pool);
    }
}
