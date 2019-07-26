#[macro_use]
extern crate diesel_derive_enum;
#[macro_use]
extern crate log;
#[macro_use]
extern crate diesel;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate lazy_static;

extern crate beancounter_grpc;
extern crate chrono;
extern crate data_encoding;
extern crate dotenv;
extern crate env_logger;
extern crate futures;
extern crate instrumented;
extern crate regex;
extern crate stripe;
extern crate tokio;
extern crate toml;
extern crate tower_hyper;
extern crate url;
extern crate yansi;

mod config;
mod models;
mod schema;
mod service;
mod sql_types;
mod stripe_client;

use beancounter_grpc::proto::server;
use futures::{Future, Stream};
use tokio::net::TcpListener;
use tower_hyper::server::{Http, Server};

fn get_db_pool(
    database: &config::Database,
) -> diesel::r2d2::Pool<diesel::r2d2::ConnectionManager<diesel::pg::PgConnection>> {
    use diesel::pg::PgConnection;
    use diesel::r2d2::{ConnectionManager, Pool};

    let manager = ConnectionManager::<PgConnection>::new(format!(
        "postgres://{}:{}@{}:{}/{}",
        database.username, database.password, database.host, database.port, database.name,
    ));

    let db_pool = Pool::builder()
        .max_size(database.connection_pool_size)
        .build(manager)
        .expect("Unable to create DB connection pool");

    let conn = db_pool.get();
    assert!(conn.is_ok());

    db_pool
}

pub fn main() {
    use std::env;

    ::env_logger::init();

    config::load_config();

    // Allow disablement of metrics reporting for testing
    if env::var_os("DISABLE_INSTRUMENTED").is_none() {
        instrumented::init(&config::CONFIG.metrics.bind_to_address);
    }

    let new_service = server::BeanCounterServer::new(service::BeanCounter::new(
        get_db_pool(&config::CONFIG.database.reader),
        get_db_pool(&config::CONFIG.database.writer),
    ));

    let mut server = Server::new(new_service);

    let http = Http::new().http2_only(true).clone();

    let addr = config::CONFIG.service.bind_to_address.parse().unwrap();
    let bind = TcpListener::bind(&addr).expect("bind");

    let serve = bind
        .incoming()
        .for_each(move |sock| {
            let addr = sock.peer_addr().ok();
            info!("New connection from addr={:?}", addr);

            let serve = server.serve_with(sock, http.clone());
            tokio::spawn(serve.map_err(|e| error!("hyper error: {:?}", e)));

            Ok(())
        })
        .map_err(|e| error!("accept error: {}", e));

    let mut rt = tokio::runtime::Builder::new()
        .core_threads(config::CONFIG.service.worker_threads)
        .build()
        .expect("Unable to build tokio runtime");

    rt.spawn(serve);
    info!(
        "Started server with {} threads, listening on {}",
        config::CONFIG.service.worker_threads,
        addr
    );
    rt.shutdown_on_idle().wait().expect("Error in main loop");
}
