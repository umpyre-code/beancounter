CREATE TABLE balances (
  id BIGSERIAL PRIMARY KEY,
  client_id UUID NOT NULL,
  created_at TIMESTAMP NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
  balance MONEY NOT NULL DEFAULT 0,
  promo_balance MONEY NOT NULL DEFAULT 0)
