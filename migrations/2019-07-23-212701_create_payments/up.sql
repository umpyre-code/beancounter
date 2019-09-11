CREATE TABLE payments (
  id BIGSERIAL PRIMARY KEY,
  created_at TIMESTAMP NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
  client_id_from UUID NOT NULL,
  client_id_to UUID NOT NULL,
  payment_cents INTEGER NOT NULL,
  message_hash TEXT NOT NULL UNIQUE)
