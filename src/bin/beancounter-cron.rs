#[macro_use]
extern crate failure;

extern crate beancounter;
extern crate chrono;
extern crate diesel;
extern crate env_logger;

use beancounter::config;
use beancounter::database;

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

pub fn main() -> Result<(), Error> {
    use std::env;

    ::env_logger::init();

    config::load_config();

    // Allow disablement of metrics reporting for testing
    if env::var_os("DISABLE_INSTRUMENTED").is_none() {
        instrumented::init(&config::CONFIG.metrics.bind_to_address);
    }

    do_cleanup()
}
