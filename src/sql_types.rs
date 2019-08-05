#[derive(Clone, Copy, Debug, PartialEq, DbEnum)]
#[PgType = "transaction_type"]
#[DieselType = "Transaction_type"]
pub enum TransactionType {
    #[db_rename = "debit"]
    Debit,
    #[db_rename = "credit"]
    Credit,
    #[db_rename = "promo_credit"]
    PromoCredit,
}

#[derive(Clone, Copy, Debug, PartialEq, DbEnum)]
#[PgType = "transaction_reason"]
#[DieselType = "Transaction_reason"]
pub enum TransactionReason {
    #[db_rename = "message_read"]
    MessageRead,
    #[db_rename = "message_unread"]
    MessageUnread,
    #[db_rename = "message_sent"]
    MessageSent,
    #[db_rename = "credit_added"]
    CreditAdded,
    #[db_rename = "payout"]
    Payout,
}
