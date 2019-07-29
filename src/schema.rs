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

    transactions (id) {
        id -> Int8,
        created_at -> Timestamp,
        client_id -> Nullable<Uuid>,
        tx_type -> Transaction_type,
        amount_cents -> Int4,
    }
}

allow_tables_to_appear_in_same_query!(balances, payments, stripe_charges, transactions,);
