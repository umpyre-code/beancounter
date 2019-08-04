CREATE TABLE stripe_connect_accounts (
  id BIGSERIAL PRIMARY KEY,
  created_at TIMESTAMP NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
  oauth_state UUID NOT NULL DEFAULT UUID_GENERATE_V4 (),
  client_id UUID UNIQUE NOT NULL,
  stripe_user_id TEXT UNIQUE,
  connect_account JSON,
  connect_credentials JSON,
  enable_automatic_payouts BOOLEAN NOT NULL DEFAULT FALSE,
  automatic_payout_threshold_cents BIGINT NOT NULL DEFAULT 10000)
