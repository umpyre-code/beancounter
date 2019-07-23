extern crate bytes;
extern crate prost;

pub mod tower_grpc {
    extern crate tower_grpc;
    pub use tower_grpc::*;
}

pub mod proto {
    extern crate tower_grpc;
    include!(concat!(env!("OUT_DIR"), "/beancounter.rs"));
}
