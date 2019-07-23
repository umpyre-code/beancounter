extern crate env_logger;
extern crate futures;
extern crate http;
extern crate hyper;
#[macro_use]
extern crate log;
extern crate beancounter_grpc;
extern crate tokio;
extern crate tower_hyper;
extern crate tower_request_modifier;
extern crate tower_service;
extern crate tower_util;
#[macro_use]
extern crate failure;

use beancounter_grpc::proto;
use beancounter_grpc::tower_grpc::Request;
use futures::Future;
use hyper::client::connect::{Destination, HttpConnector};
use std::env;
use tower_hyper::{client, util};
use tower_util::MakeService;

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "Url parser error: {}", err)]
    UrlParse { err: String },
    #[fail(display = "IO error: {}", err)]
    IoError { err: String },
    #[fail(display = "bad arguments")]
    BadArgs,
    #[fail(display = "client is not serving")]
    NotServing,
    #[fail(display = "bad response")]
    BadResponse,
}

impl From<url::ParseError> for Error {
    fn from(err: url::ParseError) -> Error {
        Error::UrlParse {
            err: format!("{}", err),
        }
    }
}

impl From<http::uri::InvalidUri> for Error {
    fn from(err: http::uri::InvalidUri) -> Error {
        Error::UrlParse {
            err: format!("{}", err),
        }
    }
}

impl From<hyper::Error> for Error {
    fn from(err: hyper::Error) -> Error {
        Error::IoError {
            err: format!("{}", err),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Error {
        Error::IoError {
            err: format!("{}", err),
        }
    }
}

pub fn main() -> Result<(), Error> {
    ::env_logger::init();

    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        error!("Usage: {} <addr>", args[0]);
        return Err(Error::BadArgs);
    }

    let address = &args[1];

    let uri: http::Uri = address.parse()?;

    let dst = Destination::try_from_uri(uri.clone())?;
    let connector = util::Connector::new(HttpConnector::new(4));
    let settings = client::Builder::new().http2_only(true).clone();
    let mut make_client = client::Connect::with_builder(connector, settings);

    let mut runtime = tokio::runtime::Runtime::new()?;

    let result = runtime.block_on(
        make_client
            .make_service(dst)
            .map_err(|e| panic!("connect error: {:?}", e))
            .and_then(move |conn| {
                use beancounter_grpc::proto::client::BeanCounter;

                let conn = tower_request_modifier::Builder::new()
                    .set_origin(uri)
                    .build(conn)
                    .unwrap();

                // Wait until the client is ready...
                BeanCounter::new(conn).ready()
            })
            .and_then(|mut client| {
                client.check(Request::new(proto::HealthCheckRequest {
                    service: "beancounter".into(),
                }))
            })
            .map(|response| response.get_ref().clone())
            .map_err(|e| {
                error!("ERR = {:?}", e);
                panic!("health check failed");
            }),
    );

    info!("{:?}", result);

    if let Ok(response) = result {
        if response.status == proto::health_check_response::ServingStatus::Serving as i32 {
            Ok(())
        } else {
            Err(Error::NotServing)
        }
    } else {
        Err(Error::BadResponse)
    }
}
