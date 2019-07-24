CREATE TABLE payments (
  id BIGSERIAL PRIMARY KEY,
  created_at TIMESTAMP NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
  client_id_from UUID,
  client_id_to UUID,
  amount_cents INTEGER NOT NULL,
  message TEXT NOT NULL UNIQUE)
