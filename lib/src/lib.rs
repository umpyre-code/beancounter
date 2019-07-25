extern crate bytes;
extern crate chrono;
extern crate prost;

pub mod tower_grpc {
    extern crate tower_grpc;
    pub use tower_grpc::*;
}

pub mod proto {
    extern crate tower_grpc;
    include!(concat!(env!("OUT_DIR"), "/beancounter.rs"));

    impl From<chrono::NaiveDateTime> for Timestamp {
        fn from(timestamp: chrono::NaiveDateTime) -> Self {
            Timestamp {
                seconds: timestamp.timestamp(),
                nanos: timestamp.timestamp_subsec_nanos() as i32,
            }
        }
    }
}
