table! {
    use diesel::sql_types::*;
    use crate::sql_types::*;

    balances (id) {
        id -> Int8,
        client_id -> Uuid,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        balance -> Money,
        promo_balance -> Money,
    }
}

table! {
    use diesel::sql_types::*;
    use crate::sql_types::*;

    transactions (id) {
        id -> Int8,
        client_id -> Nullable<Uuid>,
        created_at -> Timestamp,
        action -> Transaction_type,
        amount -> Money,
    }
}

allow_tables_to_appear_in_same_query!(balances, transactions,);
