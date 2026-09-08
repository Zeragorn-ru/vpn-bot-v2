-- VECTOR v2 clean-install baseline. No legacy data is imported.

CREATE TABLE schema_migrations (
    version text PRIMARY KEY,
    applied_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE users (
    id uuid PRIMARY KEY,
    telegram_user_id bigint UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz
);

CREATE TABLE user_profiles (
    user_id uuid PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    username text,
    first_name text,
    last_name text,
    language_code text NOT NULL DEFAULT 'ru',
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE wallets (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
    currency_code char(3) NOT NULL DEFAULT 'RUB',
    balance_minor bigint NOT NULL DEFAULT 0 CHECK (balance_minor >= 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE admin_accounts (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
    login text NOT NULL UNIQUE,
    password_hash text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    last_login_at timestamptz
);

CREATE TABLE roles (
    id uuid PRIMARY KEY,
    code text NOT NULL UNIQUE,
    name text NOT NULL
);

CREATE TABLE permissions (
    id uuid PRIMARY KEY,
    code text NOT NULL UNIQUE,
    description text NOT NULL
);

CREATE TABLE role_permissions (
    role_id uuid NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    permission_id uuid NOT NULL REFERENCES permissions(id) ON DELETE CASCADE,
    PRIMARY KEY (role_id, permission_id)
);

CREATE TABLE admin_account_roles (
    admin_account_id uuid NOT NULL REFERENCES admin_accounts(id) ON DELETE CASCADE,
    role_id uuid NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    granted_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (admin_account_id, role_id)
);

CREATE TABLE admin_sessions (
    id uuid PRIMARY KEY,
    admin_account_id uuid NOT NULL REFERENCES admin_accounts(id) ON DELETE CASCADE,
    session_hash bytea NOT NULL UNIQUE,
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz
);
CREATE INDEX admin_sessions_active_idx ON admin_sessions(session_hash, expires_at) WHERE revoked_at IS NULL;

CREATE TABLE app_settings (
    key text PRIMARY KEY,
    value jsonb NOT NULL,
    updated_by_admin_id uuid REFERENCES admin_accounts(id),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE app_secrets (
    key text PRIMARY KEY,
    value bytea NOT NULL,
    updated_by_admin_id uuid REFERENCES admin_accounts(id),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE admin_audit_log (
    id uuid PRIMARY KEY,
    actor_admin_id uuid REFERENCES admin_accounts(id),
    action text NOT NULL,
    target_type text NOT NULL,
    target_id uuid,
    before_value jsonb,
    after_value jsonb,
    correlation_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX admin_audit_log_created_idx ON admin_audit_log(created_at DESC);

CREATE TABLE catalog_tariffs (
    id uuid PRIMARY KEY,
    code text NOT NULL UNIQUE,
    name jsonb NOT NULL,
    description jsonb NOT NULL DEFAULT '{}'::jsonb,
    duration_seconds bigint CHECK (duration_seconds IS NULL OR duration_seconds > 0),
    traffic_bytes bigint CHECK (traffic_bytes IS NULL OR traffic_bytes >= 0),
    amount_minor bigint NOT NULL CHECK (amount_minor >= 0),
    currency_code char(3) NOT NULL DEFAULT 'RUB',
    is_active boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE orders (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id),
    tariff_id uuid REFERENCES catalog_tariffs(id),
    status text NOT NULL CHECK (status IN ('pending', 'paid', 'cancelled', 'fulfilled')),
    idempotency_key uuid NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE invoices (
    id uuid PRIMARY KEY,
    order_id uuid REFERENCES orders(id),
    user_id uuid NOT NULL REFERENCES users(id),
    provider text NOT NULL,
    provider_invoice_id text,
    purpose text NOT NULL CHECK (purpose IN ('wallet_top_up', 'direct_purchase')),
    status text NOT NULL CHECK (status IN ('pending', 'paid', 'expired', 'cancelled')),
    currency_code char(3) NOT NULL,
    amount_minor bigint NOT NULL CHECK (amount_minor >= 0),
    expires_at timestamptz NOT NULL,
    paid_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (provider, provider_invoice_id)
);

CREATE TABLE payment_attempts (
    id uuid PRIMARY KEY,
    invoice_id uuid NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,
    provider text NOT NULL,
    provider_payment_id text,
    status text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (provider, provider_payment_id)
);

CREATE TABLE payment_webhook_events (
    id uuid PRIMARY KEY,
    provider text NOT NULL,
    provider_event_id text NOT NULL,
    received_at timestamptz NOT NULL DEFAULT now(),
    processed_at timestamptz,
    payload jsonb NOT NULL,
    processing_error text,
    UNIQUE (provider, provider_event_id)
);

CREATE TABLE wallet_transactions (
    id uuid PRIMARY KEY,
    wallet_id uuid NOT NULL REFERENCES wallets(id),
    amount_minor bigint NOT NULL,
    currency_code char(3) NOT NULL,
    kind text NOT NULL,
    reference_type text NOT NULL,
    reference_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (reference_type, reference_id, kind)
);

CREATE TABLE subscriptions (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id),
    tariff_id uuid REFERENCES catalog_tariffs(id),
    status text NOT NULL CHECK (status IN ('provisioning_pending', 'active', 'suspended', 'expired', 'failed')),
    starts_at timestamptz,
    expires_at timestamptz,
    traffic_used_bytes bigint NOT NULL DEFAULT 0 CHECK (traffic_used_bytes >= 0),
    traffic_limit_bytes bigint CHECK (traffic_limit_bytes IS NULL OR traffic_limit_bytes >= 0),
    provider_account_id text,
    source_invoice_id uuid UNIQUE REFERENCES invoices(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX subscriptions_user_idx ON subscriptions(user_id, created_at DESC);

CREATE TABLE subscription_access_tokens (
    id uuid PRIMARY KEY,
    subscription_id uuid NOT NULL REFERENCES subscriptions(id) ON DELETE CASCADE,
    token_hash bytea NOT NULL UNIQUE,
    issued_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz,
    revoked_at timestamptz,
    last_used_at timestamptz
);
CREATE INDEX subscription_access_tokens_active_idx ON subscription_access_tokens(token_hash) WHERE revoked_at IS NULL;

CREATE TABLE provider_snapshots (
    id uuid PRIMARY KEY,
    subscription_id uuid NOT NULL UNIQUE REFERENCES subscriptions(id) ON DELETE CASCADE,
    profile bytea NOT NULL,
    provider_revision text,
    fetched_at timestamptz NOT NULL,
    fresh_until timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE subscription_request_logs (
    id uuid PRIMARY KEY,
    token_hash bytea NOT NULL,
    format text NOT NULL,
    client_category text NOT NULL,
    status_code smallint NOT NULL,
    node_count integer,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX subscription_request_logs_created_idx ON subscription_request_logs(created_at DESC);

CREATE TABLE outbox_events (
    id uuid PRIMARY KEY,
    aggregate_type text NOT NULL,
    aggregate_id uuid NOT NULL,
    event_type text NOT NULL,
    payload jsonb NOT NULL,
    attempts integer NOT NULL DEFAULT 0,
    available_at timestamptz NOT NULL DEFAULT now(),
    locked_at timestamptz,
    lock_owner text,
    processed_at timestamptz,
    last_error text,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX outbox_pending_idx ON outbox_events(available_at, created_at) WHERE processed_at IS NULL;

CREATE TABLE idempotency_keys (
    key uuid PRIMARY KEY,
    scope text NOT NULL,
    request_hash bytea NOT NULL,
    response_status smallint,
    response_body jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL
);

CREATE TABLE telegram_updates (
    update_id bigint PRIMARY KEY,
    claimed_at timestamptz NOT NULL DEFAULT now(),
    processed_at timestamptz,
    processing_error text,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE required_channels (
    id uuid PRIMARY KEY,
    telegram_chat_id bigint NOT NULL UNIQUE,
    title text NOT NULL,
    public_url text,
    is_active boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

INSERT INTO roles (id, code, name) VALUES
    ('01900000-0000-7000-8000-000000000001', 'owner', 'Owner'),
    ('01900000-0000-7000-8000-000000000002', 'admin', 'Administrator'),
    ('01900000-0000-7000-8000-000000000003', 'support', 'Support'),
    ('01900000-0000-7000-8000-000000000004', 'billing', 'Billing'),
    ('01900000-0000-7000-8000-000000000005', 'operator', 'Operator'),
    ('01900000-0000-7000-8000-000000000006', 'viewer', 'Viewer');

INSERT INTO permissions (id, code, description) VALUES
    ('01900000-0000-7000-8100-000000000001', 'system.manage', 'Manage runtime and integrations'),
    ('01900000-0000-7000-8100-000000000002', 'secrets.manage', 'Write encrypted integration secrets'),
    ('01900000-0000-7000-8100-000000000003', 'users.read', 'Read users and subscriptions'),
    ('01900000-0000-7000-8100-000000000004', 'billing.manage', 'Manage tariffs and payments'),
    ('01900000-0000-7000-8100-000000000005', 'operations.manage', 'Manage provisioning operations'),
    ('01900000-0000-7000-8100-000000000006', 'audit.read', 'Read audit history');

INSERT INTO role_permissions (role_id, permission_id)
SELECT roles.id, permissions.id FROM roles CROSS JOIN permissions WHERE roles.code = 'owner';
INSERT INTO role_permissions (role_id, permission_id)
SELECT roles.id, permissions.id FROM roles JOIN permissions ON permissions.code IN ('users.read', 'billing.manage', 'operations.manage', 'audit.read') WHERE roles.code = 'admin';
INSERT INTO role_permissions (role_id, permission_id)
SELECT roles.id, permissions.id FROM roles JOIN permissions ON permissions.code IN ('users.read') WHERE roles.code = 'support';
INSERT INTO role_permissions (role_id, permission_id)
SELECT roles.id, permissions.id FROM roles JOIN permissions ON permissions.code IN ('billing.manage') WHERE roles.code = 'billing';
INSERT INTO role_permissions (role_id, permission_id)
SELECT roles.id, permissions.id FROM roles JOIN permissions ON permissions.code IN ('users.read', 'operations.manage') WHERE roles.code = 'operator';
INSERT INTO role_permissions (role_id, permission_id)
SELECT roles.id, permissions.id FROM roles JOIN permissions ON permissions.code IN ('users.read', 'audit.read') WHERE roles.code = 'viewer';

INSERT INTO schema_migrations(version) VALUES ('001_baseline');
