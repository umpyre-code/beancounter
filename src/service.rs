use beancounter_grpc::proto;
use beancounter_grpc::proto::*;
use beancounter_grpc::tower_grpc::{Code, Request, Response, Status};
use diesel::prelude::*;
use futures::future::FutureResult;
use instrumented::{instrument, prometheus, register};

use crate::schema;

fn make_intcounter(name: &str, description: &str) -> prometheus::IntCounter {
    let counter = prometheus::IntCounter::new(name, description).unwrap();
    register(Box::new(counter.clone())).unwrap();
    counter
}

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
    #[fail(display = "invalid client_id: {}", err)]
    InvalidClientId { err: String },
    #[fail(display = "Bad arguments specified for request")]
    BadArguments,
}

impl From<uuid::parser::ParseError> for RequestError {
    fn from(err: uuid::parser::ParseError) -> RequestError {
        RequestError::InvalidClientId {
            err: format!("{}", err),
        }
    }
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
        Err(RequestError::BadArguments)
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
        Err(RequestError::BadArguments)
    }

    #[instrument(INFO)]
    fn handle_withdraw_credits(
        &self,
        request: &WithdrawCreditsRequest,
    ) -> Result<WithdrawCreditsResponse, RequestError> {
        Err(RequestError::BadArguments)
    }
}

impl proto::server::BeanCounter for BeanCounter {
    type GetBalancesFuture = FutureResult<Response<GetBalancesResponse>, Status>;
    type GetTransactionsFuture = FutureResult<Response<GetTransactionsResponse>, Status>;
    type AddCreditsFuture = FutureResult<Response<AddCreditsResponse>, Status>;
    type WithdrawCreditsFuture = FutureResult<Response<WithdrawCreditsResponse>, Status>;
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
    use futures::future;
    use std::sync::Mutex;

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

        empty_tables![transactions, balances];
    }

    #[test]
    fn test_hello_world() {
        let _lock = LOCK.lock().unwrap();

        tokio::run(future::lazy(|| {
            let (db_pool,) = get_pools();

            empty_tables(&db_pool);

            let beancounter = BeanCounter::new(db_pool.clone(), db_pool.clone());

            future::ok(())
        }));
    }
}
