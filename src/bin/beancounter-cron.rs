#[macro_use]
extern crate diesel;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate log;

extern crate beancounter;
extern crate chrono;
extern crate env_logger;

use beancounter::config;
use beancounter::database;
use diesel::sql_types::*;
use uuid::Uuid;

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "database error: {}", err)]
    DatabaseError { err: String },
}

impl From<diesel::result::Error> for Error {
    fn from(err: diesel::result::Error) -> Self {
        Self::DatabaseError {
            err: err.to_string(),
        }
    }
}

#[derive(Debug, QueryableByName)]
pub struct ClientPayout {
    #[sql_type = "diesel::pg::types::sql_types::Uuid"]
    pub client_id: Uuid,
    #[sql_type = "BigInt"]
    pub withdrawable_cents: i64,
    #[sql_type = "Bool"]
    pub enable_automatic_payouts: bool,
    #[sql_type = "BigInt"]
    pub automatic_payout_threshold_cents: i64,
    #[sql_type = "Nullable<Text>"]
    pub stripe_user_id: Option<String>,
}

fn do_cleanup() -> Result<(), Error> {
    use beancounter::models::Payment;
    use beancounter::schema::payments::dsl::*;
    use beancounter::service::add_transaction;
    use beancounter::sql_types::TransactionReason;
    use chrono::{Duration, Utc};
    use diesel::connection::Connection;
    use diesel::prelude::*;

    let db_pool = database::get_db_pool(&config::CONFIG.database.writer);

    let conn = db_pool.get().unwrap();

    let now = Utc::now().naive_utc();
    let thirty_days_ago = now - Duration::days(30);

    conn.transaction::<_, Error, _>(|| {
        let expired_payments: Vec<Payment> = payments
            .filter(created_at.lt(thirty_days_ago))
            .get_results(&conn)?;

        for payment in expired_payments.iter() {
            // This payment was never settled. Refund (credit) the fee to the sender.
            add_transaction(
                Some(payment.client_id_from),
                None,
                payment.payment_cents,
                TransactionReason::MessageUnread,
                &conn,
            )?;

            // Delete the payment record from the DB
            diesel::delete(payments)
                .filter(id.eq(payment.id))
                .execute(&conn)?;
        }

        Ok(())
    })?;

    Ok(())
}

fn do_payouts() -> Result<(), Error> {
    use beancounter_grpc::proto::ConnectPayoutRequest;
    use diesel::prelude::*;
    use diesel::sql_query;

    let db_pool_reader = database::get_db_pool(&config::CONFIG.database.reader);
    let db_pool_writer = database::get_db_pool(&config::CONFIG.database.writer);
    let beancounter =
        beancounter::service::BeanCounter::new(db_pool_reader.clone(), db_pool_writer.clone());

    let reader_conn = db_pool_reader.get().unwrap();

    let payout_results: Vec<ClientPayout> = sql_query(
        r#"
        SELECT
            b.client_id,
            b.withdrawable_cents,
            a.enable_automatic_payouts,
            a.automatic_payout_threshold_cents,
            a.stripe_user_id
        FROM
            balances AS b
            INNER JOIN stripe_connect_accounts AS a ON b.client_id = a.client_id
        WHERE
            withdrawable_cents >= a.automatic_payout_threshold_cents
            AND a.enable_automatic_payouts = TRUE
            AND NOT EXISTS (
                SELECT
                    *
                FROM
                    stripe_connect_transfers AS t
                WHERE
                    t.created_at >= NOW() - interval '24 hours'
                    AND b.client_id = t.client_id);
           "#,
    )
    .load(&reader_conn)?;

    info!("{} payouts to process", payout_results.len());

    for payout in payout_results.iter() {
        let payout = beancounter.handle_connect_payout(&ConnectPayoutRequest {
            client_id: payout.client_id.to_simple().to_string(),
            amount_cents: payout.withdrawable_cents as i32,
        });

        match payout {
            Ok(payout) => info!("Payout: {:?}", payout),
            Err(err) => error!("Payout error: {:?}", err),
        }
    }

    Ok(())
}

pub fn main() -> Result<(), Error> {
    use std::env;

    ::env_logger::init();

    config::load_config();

    // Allow disablement of metrics reporting for testing
    if env::var_os("DISABLE_INSTRUMENTED").is_none() {
        instrumented::init(&config::CONFIG.metrics.bind_to_address);
    }

    do_cleanup()?;
    do_payouts()?;

    Ok(())
}
