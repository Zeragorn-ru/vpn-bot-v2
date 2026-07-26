CREATE TABLE IF NOT EXISTS required_channels (
  id uuid PRIMARY KEY,
  telegram_chat_id bigint NOT NULL UNIQUE,
  title text NOT NULL,
  public_url text,
  is_active boolean NOT NULL DEFAULT true,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);
