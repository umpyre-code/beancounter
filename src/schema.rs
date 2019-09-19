table! {
    use diesel::sql_types::*;
    use crate::sql_types::*;

    balances (id) {
        id -> Int8,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        client_id -> Uuid,
        balance_cents -> Int8,
        promo_cents -> Int8,
        withdrawable_cents -> Int8,
    }
}

table! {
    use diesel::sql_types::*;
    use crate::sql_types::*;

    payments (id) {
        id -> Int8,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        client_id_from -> Uuid,
        client_id_to -> Uuid,
        payment_cents -> Int4,
        message_hash -> Text,
        is_promo -> Bool,
    }
}

table! {
    use diesel::sql_types::*;
    use crate::sql_types::*;

    stripe_charges (id) {
        id -> Int8,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        client_id -> Uuid,
        token -> Json,
        charge -> Json,
    }
}

table! {
    use diesel::sql_types::*;
    use crate::sql_types::*;

    stripe_connect_accounts (id) {
        id -> Int8,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        oauth_state -> Uuid,
        client_id -> Uuid,
        stripe_user_id -> Nullable<Text>,
        connect_account -> Nullable<Json>,
        connect_credentials -> Nullable<Json>,
        enable_automatic_payouts -> Bool,
        automatic_payout_threshold_cents -> Int8,
    }
}

table! {
    use diesel::sql_types::*;
    use crate::sql_types::*;

    stripe_connect_transfers (id) {
        id -> Int8,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        client_id -> Uuid,
        stripe_user_id -> Text,
        connect_transfer -> Json,
        amount_cents -> Int4,
    }
}

table! {
    use diesel::sql_types::*;
    use crate::sql_types::*;

    transactions (id) {
        id -> Int8,
        created_at -> Timestamp,
        client_id -> Nullable<Uuid>,
        tx_type -> Transaction_type,
        tx_reason -> Transaction_reason,
        amount_cents -> Int4,
    }
}

allow_tables_to_appear_in_same_query!(
    balances,
    payments,
    stripe_charges,
    stripe_connect_accounts,
    stripe_connect_transfers,
    transactions,
);
