extern crate bigdecimal;

use beancounter_grpc::proto;
use beancounter_grpc::proto::*;
use beancounter_grpc::tower_grpc::{Code, Request, Response, Status};
use bigdecimal::BigDecimal;
use diesel::prelude::*;
use futures::future::FutureResult;
use instrumented::{instrument, prometheus, register};

use crate::models;
use crate::schema;

fn make_intcounter(name: &str, description: &str) -> prometheus::IntCounter {
    let counter = prometheus::IntCounter::new(name, description).unwrap();
    register(Box::new(counter.clone())).unwrap();
    counter
}

// This amount is calculated by subtracting Stripe's maximum fee of 3.9% + 30C
// from their charge maximum, which is $999,999.99 according to
// https://stripe.com/docs/currencies#minimum-and-maximum-charge-amounts.
// The base 2.9% + 30C fee also includes an optional fee of 1% for foreign cards.
// Thus, it's calculated like so (w/ Python):
//   >>> (99999999 - 30) / 1.039
//   96246360.92396536
static MAX_PAYMENT_AMOUNT: i32 = 96246360;

lazy_static! {
    static ref GET_BALANCE: prometheus::IntCounter =
        make_intcounter("get_balance", "get_balance called");
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

fn calculate_balances(credit_sum: i64, promo_credit_sum: i64, debit_sum: i64) -> (i64, i64) {
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
        .filter(tx_type.eq(TransactionType::Credit))
        .filter(client_id.eq(client_uuid))
        .select(sum(amount_cents))
        .first::<Option<i64>>(conn)?
        .unwrap_or_else(|| 0);

    let promo_credit_sum = transactions
        .filter(tx_type.eq(TransactionType::PromoCredit))
        .filter(client_id.eq(client_uuid))
        .select(sum(amount_cents))
        .first::<Option<i64>>(conn)?
        .unwrap_or_else(|| 0);

    let debit_sum = transactions
        .filter(tx_type.eq(TransactionType::Debit))
        .filter(client_id.eq(client_uuid))
        .select(sum(amount_cents))
        .first::<Option<i64>>(conn)?
        .unwrap_or_else(|| 0);

    let (balance_cents_remaining, promo_cents_remaining) =
        calculate_balances(credit_sum, promo_credit_sum, debit_sum);

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
    fn handle_get_balances(
        &self,
        request: &GetBalancesRequest,
    ) -> Result<GetBalancesResponse, RequestError> {
        use crate::models::*;
        use crate::sql_types::*;
        use diesel::dsl::*;
        use diesel::insert_into;
        use diesel::prelude::*;
        use diesel::result::Error;
        use schema::balances::columns::*;
        use schema::balances::table as balances;
        use uuid::Uuid;

        let client_uuid = Uuid::parse_str(&request.client_id)?;

        let conn = self.db_reader.get().unwrap();
        let balance = conn.transaction::<Balance, Error, _>(|| {
            let result = balances.filter(client_id.eq(client_uuid)).first(&conn);

            match result {
                // If the balance record exists, return that
                Ok(result) => Ok(result),
                // If there's no record yet, create a new zeroed out balance record.
                Err(diesel::NotFound) => Ok(insert_into(balances)
                    .values(&NewZeroBalance {
                        client_id: client_uuid,
                    })
                    .get_result(&conn)?),
                Err(err) => Err(err),
            }
        })?;

        Ok(GetBalancesResponse {
            client_id: balance.client_id.to_simple().to_string(),
            balance_cents: balance.balance_cents,
            promo_cents: balance.promo_cents,
        })
    }

    #[instrument(INFO)]
    fn handle_get_transactions(
        &self,
        request: &GetTransactionsRequest,
    ) -> Result<GetTransactionsResponse, RequestError> {
        Err(RequestError::BadArguments)
    }

    #[instrument(INFO)]
    fn handle_add_credits(
        &self,
        request: &AddCreditsRequest,
    ) -> Result<AddCreditsResponse, RequestError> {
        use crate::models::*;
        use crate::sql_types::*;
        use diesel::prelude::*;
        use diesel::result::Error;
        use schema::transactions::table as transactions;
        use uuid::Uuid;

        let client_uuid = Uuid::parse_str(&request.client_id)?;

        let tx_credit = NewTransaction {
            client_id: Some(client_uuid),
            tx_type: TransactionType::Credit,
            amount_cents: request.amount_cents,
            settled: true,
        };
        let tx_debit = NewTransaction {
            client_id: None,
            tx_type: TransactionType::Debit,
            amount_cents: -request.amount_cents,
            settled: true,
        };
        let conn = self.db_writer.get().unwrap();
        let balance = conn.transaction::<Balance, Error, _>(|| {
            diesel::insert_into(transactions)
                .values(&tx_credit)
                .execute(&conn)?;

            diesel::insert_into(transactions)
                .values(&tx_debit)
                .execute(&conn)?;

            let balance = update_and_return_balance(client_uuid, &conn)?;

            Ok(balance)
        })?;

        Ok(AddCreditsResponse {
            client_id: client_uuid.to_simple().to_string(),
            balance_cents: balance.balance_cents,
            promo_cents: balance.promo_cents,
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
        Err(RequestError::BadArguments)
    }

    #[instrument(INFO)]
    fn handle_settle_payment(
        &self,
        request: &SettlePaymentRequest,
    ) -> Result<SettlePaymentResponse, RequestError> {
        Err(RequestError::BadArguments)
    }
}

impl proto::server::BeanCounter for BeanCounter {
    type GetBalancesFuture = FutureResult<Response<GetBalancesResponse>, Status>;
    type GetTransactionsFuture = FutureResult<Response<GetTransactionsResponse>, Status>;
    type AddCreditsFuture = FutureResult<Response<AddCreditsResponse>, Status>;
    type WithdrawCreditsFuture = FutureResult<Response<WithdrawCreditsResponse>, Status>;
    type AddPaymentFuture = FutureResult<Response<AddPaymentResponse>, Status>;
    type SettlePaymentFuture = FutureResult<Response<SettlePaymentResponse>, Status>;
    type CheckFuture = FutureResult<Response<HealthCheckResponse>, Status>;

    /// Get account balances
    fn get_balances(&mut self, request: Request<GetBalancesRequest>) -> Self::GetBalancesFuture {
        use futures::future::IntoFuture;
        self.handle_get_balances(request.get_ref())
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

    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;
    use diesel::dsl::*;
    use diesel::pg::PgConnection;
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

    #[test]
    fn test_add_credits() {
        use diesel::prelude::*;
        use schema::transactions::columns::*;
        use schema::transactions::table as transactions;

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
                amount_cents: 100,
            });

            assert!(result.is_ok());
            let result = result.unwrap();
            assert_eq!(result.balance_cents, amount);
            assert_eq!(result.promo_cents, 0);
        }

        let conn = db_pool.get().unwrap();

        let tx_count = transactions.select(count(id)).first(&conn);
        assert_eq!(Ok(200), tx_count);

        // All credits are positive, and all debits are negative. When summed,
        // they should always balance out to 0.
        let tx_sum = transactions
            .select(sum(amount_cents))
            .first::<Option<i64>>(&conn)
            .unwrap();
        assert_eq!(Some(0), tx_sum);

        for uuid in uuids.iter() {
            let balance_result = beancounter.handle_get_balances(&GetBalancesRequest {
                client_id: uuid.clone(),
            });

            assert!(balance_result.is_ok());
            let balance_result = balance_result.unwrap();
            assert_eq!(balance_result.balance_cents, 100);
            assert_eq!(balance_result.promo_cents, 0);
        }
    }

    #[test]
    fn test_calculate_balances() {
        let (balance, promo) = calculate_balances(0, 0, 0);
        assert_eq!(balance, 0);
        assert_eq!(promo, 0);

        let (balance, promo) = calculate_balances(10, 0, 0);
        assert_eq!(balance, 10);
        assert_eq!(promo, 0);

        let (balance, promo) = calculate_balances(10, 0, -10);
        println!("{} {}", balance, promo);
        assert_eq!(balance, 0);
        assert_eq!(promo, 0);

        let (balance, promo) = calculate_balances(10, 10, -10);
        assert_eq!(balance, 10);
        assert_eq!(promo, 0);

        let (balance, promo) = calculate_balances(10, 10, -20);
        assert_eq!(balance, 0);
        assert_eq!(promo, 0);

        let (balance, promo) = calculate_balances(0, 10, -10);
        assert_eq!(balance, 0);
        assert_eq!(promo, 0);

        // These cases (negative balances) should never occur, but we test for
        // it here anyway, just to make sure the math is right.
        let (balance, promo) = calculate_balances(0, 10, -20);
        assert_eq!(balance, -10);
        assert_eq!(promo, 0);

        let (balance, promo) = calculate_balances(10, 0, -20);
        assert_eq!(balance, -10);
        assert_eq!(promo, 0);
    }
}
