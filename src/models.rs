extern crate uuid;

use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::schema::*;
use crate::sql_types::*;

#[derive(Queryable, Identifiable)]
pub struct Transaction {
    pub id: i64,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub client_id: Option<Uuid>,
    pub tx_type: TransactionType,
    pub amount_cents: i32,
    pub settled: bool,
}

#[derive(Insertable)]
#[table_name = "transactions"]
pub struct NewTransaction {
    pub client_id: Option<Uuid>,
    pub tx_type: TransactionType,
    pub amount_cents: i32,
    pub settled: bool,
}

#[derive(Queryable, Identifiable, Debug)]
pub struct Balance {
    pub id: i64,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub client_id: Uuid,
    pub balance_cents: i64,
    pub promo_cents: i64,
}

#[derive(Insertable)]
#[table_name = "balances"]
pub struct NewBalance {
    pub client_id: Uuid,
    pub balance_cents: i64,
    pub promo_cents: i64,
}

#[derive(Insertable)]
#[table_name = "balances"]
pub struct NewZeroBalance {
    pub client_id: Uuid,
}

#[derive(AsChangeset)]
#[table_name = "balances"]
pub struct UpdatedBalance {
    pub balance_cents: i64,
    pub promo_cents: i64,
}
