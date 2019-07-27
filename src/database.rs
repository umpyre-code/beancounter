use crate::config;

pub fn get_db_pool(
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
