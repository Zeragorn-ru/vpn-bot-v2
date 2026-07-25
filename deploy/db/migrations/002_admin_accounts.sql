CREATE TABLE IF NOT EXISTS admin_accounts (
  user_id uuid PRIMARY KEY REFERENCES users(id),
  login text NOT NULL UNIQUE,
  password_hash text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  last_login_at timestamptz
);
