CREATE TABLE balances (
  id BIGSERIAL PRIMARY KEY,
  created_at TIMESTAMP NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
  client_id UUID UNIQUE NOT NULL,
  balance_cents BIGINT NOT NULL DEFAULT 0,
  promo_cents BIGINT NOT NULL DEFAULT 0,
  withdrawable_cents BIGINT NOT NULL DEFAULT 0)
