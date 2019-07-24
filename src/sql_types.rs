#[derive(Debug, PartialEq, DbEnum)]
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
