-- 0001_init.sql: schema from docs/DESIGN.md 5.9, verbatim.
create extension if not exists pgcrypto;

create type plan_t as enum ('lifetime', 'yearly');
create type ent_status_t as enum ('active', 'past_due', 'ended', 'revoked');

create table customers (
  id uuid primary key default gen_random_uuid(),
  paddle_customer_id text unique,
  email text not null,                          -- lowercased, trimmed
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);
create index customers_email_idx on customers (email);

create table entitlements (
  id text primary key,                          -- 'ent_' + ULID
  customer_id uuid not null references customers(id),
  plan plan_t not null,
  status ent_status_t not null,
  product_key_hash bytea not null unique,
  product_key_enc text not null,
  key4 text not null,
  max_devices int not null default 3,
  max_web int not null default 3,
  paddle_transaction_id text unique,            -- purchase transaction
  paddle_subscription_id text unique,           -- yearly only
  paddle_price_id text not null,
  access_until timestamptz,                     -- null for lifetime
  cancel_at timestamptz,
  revoked_at timestamptz,
  revoked_reason text check (revoked_reason in ('refund', 'chargeback', 'manual')),
  last_event_at timestamptz,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  constraint yearly_has_sub check ((plan = 'yearly') = (paddle_subscription_id is not null))
);
create index entitlements_customer_idx on entitlements (customer_id);

create table devices (
  id uuid primary key default gen_random_uuid(),
  entitlement_id text not null references entitlements(id) on delete cascade,
  kind text not null check (kind in ('desktop', 'web')),
  device_hash text not null,
  device_name text,
  platform text check (platform in ('windows', 'macos', 'linux', 'web')),
  app_version text,
  activated_at timestamptz not null default now(),
  last_seen_at timestamptz not null default now(),
  deactivated_at timestamptz,
  deactivated_by text check (deactivated_by in ('device', 'dashboard', 'idle', 'evicted', 'support'))
);
create unique index devices_active_uq on devices (entitlement_id, device_hash) where deactivated_at is null;

create table claims (
  id text primary key,                          -- 'clm_' + ULID
  secret_hash bytea not null,
  kind text not null check (kind in ('desktop', 'web')),
  device_hash text not null,
  device_name text,
  platform text,
  entitlement_id text references entitlements(id),
  status text not null default 'pending' check (status in ('pending', 'fulfilled', 'expired')),
  created_at timestamptz not null default now(),
  expires_at timestamptz not null
);

create table checkouts (
  id text primary key,                          -- 'chk_' + ULID
  secret_hash bytea not null,
  plan plan_t not null,
  paddle_price_id text not null,
  paddle_transaction_id text unique,
  claim_id text references claims(id),
  entitlement_id text references entitlements(id),
  created_at timestamptz not null default now()
);

create table paddle_events (
  event_id text primary key,
  event_type text not null,
  occurred_at timestamptz not null,
  payload jsonb not null,
  received_at timestamptz not null default now(),
  status text not null default 'received' check (status in ('received', 'processed', 'ignored', 'error')),
  attempts int not null default 0,
  last_error text,
  processed_at timestamptz
);
create index paddle_events_status_idx on paddle_events (status, received_at);

create table applied_adjustments (
  adjustment_id text primary key,
  entitlement_id text not null references entitlements(id),
  action text not null,
  applied_at timestamptz not null default now()
);

create table otp_codes (
  email text primary key,
  code_hmac bytea not null,                     -- HMAC-SHA256(OTP_PEPPER, email || ':' || code)
  sent_at timestamptz not null,
  expires_at timestamptz not null,
  attempts int not null default 0,
  used_at timestamptz
);

create table sessions (
  id_hash bytea primary key,                    -- sha256(cookie value)
  email text not null,
  created_at timestamptz not null default now(),
  last_used_at timestamptz not null default now(),
  expires_at timestamptz not null,
  revoked_at timestamptz,
  user_agent text
);

create table email_log (
  id uuid primary key default gen_random_uuid(),
  kind text not null,
  ref text not null,
  to_email text not null,
  resend_id text,
  sent_at timestamptz not null default now(),
  unique (kind, ref)
);

create table audit_log (
  id bigserial primary key,
  at timestamptz not null default now(),
  actor text not null,                          -- 'webhook', 'api', 'dashboard', 'support', 'cron'
  action text not null,
  entitlement_id text,
  detail jsonb
);

create table rate_limits (
  key text not null,
  window_start timestamptz not null,
  count int not null default 0,
  primary key (key, window_start)
);

alter table customers enable row level security;
alter table entitlements enable row level security;
alter table devices enable row level security;
alter table claims enable row level security;
alter table checkouts enable row level security;
alter table paddle_events enable row level security;
alter table applied_adjustments enable row level security;
alter table otp_codes enable row level security;
alter table sessions enable row level security;
alter table email_log enable row level security;
alter table audit_log enable row level security;
alter table rate_limits enable row level security;
-- No policies: only the service role (which bypasses RLS) can read or write.

-- Added for the routes (not in the DESIGN.md listing):
create index sessions_email_idx on sessions (email);
create index devices_entitlement_active_idx on devices (entitlement_id) where deactivated_at is null;
create index claims_pending_idx on claims (expires_at) where status = 'pending';
create index rate_limits_window_idx on rate_limits (window_start);
