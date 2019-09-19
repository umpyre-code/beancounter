extern crate uuid;

use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::schema::*;
use crate::sql_types::*;

#[derive(Debug, Queryable, Identifiable)]
pub struct Transaction {
    pub id: i64,
    pub created_at: NaiveDateTime,
    pub client_id: Option<Uuid>,
    pub tx_type: TransactionType,
    pub tx_reason: TransactionReason,
    pub amount_cents: i32,
}

#[derive(Insertable)]
#[table_name = "transactions"]
pub struct NewTransaction {
    pub client_id: Option<Uuid>,
    pub tx_type: TransactionType,
    pub tx_reason: TransactionReason,
    pub amount_cents: i32,
}

#[derive(Queryable, Identifiable, Debug)]
pub struct Balance {
    pub id: i64,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub client_id: Uuid,
    pub balance_cents: i64,
    pub promo_cents: i64,
    pub withdrawable_cents: i64,
}

#[derive(Insertable)]
#[table_name = "balances"]
pub struct NewBalance {
    pub client_id: Uuid,
    pub balance_cents: i64,
    pub promo_cents: i64,
    pub withdrawable_cents: i64,
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
    pub withdrawable_cents: i64,
}

#[derive(Queryable, Identifiable)]
pub struct Payment {
    pub id: i64,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub client_id_from: Uuid,
    pub client_id_to: Uuid,
    pub payment_cents: i32,
    pub message_hash: String,
    pub is_promo: bool,
}

#[derive(Insertable)]
#[table_name = "payments"]
pub struct NewPayment {
    pub client_id_from: Uuid,
    pub client_id_to: Uuid,
    pub payment_cents: i32,
    pub message_hash: String,
    pub is_promo: bool,
}

#[derive(Queryable, Identifiable)]
pub struct StripeCharge {
    pub id: i64,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub client_id: Uuid,
    pub charge: serde_json::Value,
}

#[derive(Insertable)]
#[table_name = "stripe_charges"]
pub struct NewStripeCharge {
    pub client_id: Uuid,
    pub charge: serde_json::Value,
}

#[derive(Debug, Queryable, Identifiable)]
pub struct StripeConnectAccount {
    pub id: i64,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub oauth_state: Uuid,
    pub client_id: Uuid,
    pub stripe_user_id: Option<String>,
    pub connect_account: Option<serde_json::Value>,
    pub connect_credentials: Option<serde_json::Value>,
    pub enable_automatic_payouts: bool,
    pub automatic_payout_threshold_cents: i64,
}

#[derive(Insertable)]
#[table_name = "stripe_connect_accounts"]
pub struct NewStripeConnectAccount {
    pub client_id: Uuid,
}

#[derive(Debug, AsChangeset)]
#[table_name = "stripe_connect_accounts"]
pub struct UpdateStripeConnectAccountPrefs {
    pub enable_automatic_payouts: bool,
    pub automatic_payout_threshold_cents: i64,
}

#[derive(Debug, AsChangeset)]
#[table_name = "stripe_connect_accounts"]
pub struct UpdateStripeConnectAccount {
    pub stripe_user_id: Option<String>,
    pub connect_account: Option<serde_json::Value>,
    pub connect_credentials: Option<serde_json::Value>,
}

#[derive(Debug, Queryable, Identifiable)]
pub struct StripeConnectTransfer {
    pub id: i64,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub client_id: Uuid,
    pub stripe_user_id: String,
    pub connect_transfer: serde_json::Value,
    pub amount_cents: i32,
}

#[derive(Insertable)]
#[table_name = "stripe_connect_transfers"]
pub struct NewStripeConnectTransfer {
    pub client_id: Uuid,
    pub stripe_user_id: String,
    pub connect_transfer: serde_json::Value,
    pub amount_cents: i32,
}
