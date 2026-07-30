CREATE TABLE IF NOT EXISTS app_secrets (
  key text PRIMARY KEY,
  value bytea NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now()
);
