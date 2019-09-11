CREATE TABLE stripe_connect_transfers (
  id BIGSERIAL PRIMARY KEY,
  created_at TIMESTAMP NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
  client_id UUID NOT NULL,
  stripe_user_id TEXT NOT NULL,
  connect_transfer JSON NOT NULL,
  amount_cents INT NOT NULL)
