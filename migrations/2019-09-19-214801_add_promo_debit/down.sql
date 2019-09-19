ALTER TYPE TRANSACTION_TYPE RENAME TO TRANSACTION_TYPE_OLD;

CREATE TYPE TRANSACTION_TYPE AS ENUM (
  'debit',
  'credit',
  'promo_credit'
);

ALTER TABLE transactions
  ALTER COLUMN tx_type TYPE TRANSACTION_TYPE
  USING tx_type::text::TRANSACTION_TYPE;

DROP TYPE TRANSACTION_TYPE_OLD;

