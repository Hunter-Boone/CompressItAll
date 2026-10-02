# @cia/site

The marketing site and the licence/payment API: Next.js 15 (App Router, Node runtime) on Vercel. Specification: `docs/DESIGN.md` section 5 (auth, licensing, payments), 6.3 (updater route), 6.5 (hosting).

```
app/            pages (/, /buy, /success, /account, /download, /help, /privacy, /terms, /refunds)
app/api/v1/     route handlers, one folder per row of DESIGN.md 5.10
lib/            env validation, Db interface (Supabase + in-memory), Paddle, signature, tokens, keys, email, sessions, OTP, rate limits, webhook processing, updater logic
emails/         the six templates from DESIGN.md 5.11 plus the dead-letter alert (plain HTML strings)
supabase/       migrations 0001_init.sql and 0002_functions.sql; schema.sql is a generated dump
test/           Vitest suites; test/fixtures/paddle/*.json are sandbox-shaped payloads signed in-test
```

## Local development

```
cp apps/site/.env.example apps/site/.env.local
# fill in real values, or keep SMG_DB=memory and replace only the secrets:
#   LICENSE_SIGNING_KEY: the placeholder seed from crates/cia-license/test-keys.json (kid 2026-10) works for dev
#   KEY_ENCRYPTION_KEY, OTP_PEPPER: openssl rand -base64 32
#   IP_HASH_SALT: openssl rand -base64 16
#   CRON_SECRET: any random string
npm run dev -w apps/site          # http://localhost:3031
```

`SMG_DB=memory` runs every API route against an in-memory database (`lib/db-memory.ts`); it is wiped on restart and is what the tests use. Anything else needs `SUPABASE_URL` and `SUPABASE_SERVICE_ROLE_KEY`. `lib/env.ts` validates every variable on the first request and refuses placeholders and sandbox/production mismatches; `next build` never reads secrets.

The Paddle webhook cannot reach a laptop. For end-to-end sandbox testing, deploy a preview (`vercel`) and point the sandbox notification destination at `https://<preview>/api/v1/webhooks/paddle`, or replay a signed fixture with `test/helpers.ts`'s `deliver` in a test.

If the dev server is left running for others, register it on the Links dashboard as "Smidge site (dev)" (`register-webapp` skill).

## Scripts

| Command | What |
|---|---|
| `npm run dev -w apps/site` | dev server on 3031 |
| `npm test -w apps/site` | Vitest (MemoryDb, fetch stubbed; no network) |
| `npm run typecheck -w apps/site` | `tsc --noEmit` |
| `npm run build -w apps/site` | `next build`; works with no env at all |
| `npm run lint` (repo root) | eslint, includes this app |

## Paddle sandbox setup (Hunter)

1. In the sandbox dashboard create one product, "Smidge Pro", with two prices: a one-time price (Lifetime) and a yearly recurring price with **no trial period** (Yearly). Put the two `pri_...` ids in `PADDLE_PRICE_ID_LIFETIME` and `PADDLE_PRICE_ID_YEARLY`.
2. Developer tools, Authentication: create an API key (`pdl_sdbx_apikey_...`) with read/write on transactions, customers, subscriptions, adjustments and customer portal sessions. Put it in `PADDLE_API_KEY`.
3. Developer tools, Authentication: create a client-side token (`test_...`) for `NEXT_PUBLIC_PADDLE_CLIENT_TOKEN`. Set `PADDLE_ENV` and `NEXT_PUBLIC_PADDLE_ENV` to `sandbox`.
4. Developer tools, Notifications: add a destination of type webhook, URL `https://www.<domain>/api/v1/webhooks/paddle` (or the preview URL), subscribed to exactly these events:
   `transaction.completed`, `subscription.created`, `subscription.activated`, `subscription.updated`, `subscription.past_due`, `subscription.paused`, `subscription.resumed`, `subscription.canceled`, `subscription.trialing`, `adjustment.created`, `adjustment.updated`, `customer.updated`.
   Copy the destination's secret key (`pdl_ntfset_...`) into `PADDLE_WEBHOOK_SECRET`.
5. Checkout settings: add the site domain to the approved domains list, and set the default payment link to `https://www.<domain>/buy`.
6. Production is the same four steps in the live dashboard with `pdl_live_` / `live_` credentials and `PADDLE_ENV=production`. `lib/env.ts` refuses mixed prefixes. Paddle's domain review needs `/terms`, `/privacy` and `/refunds` to carry the legal seller name; those pages have `TODO(Hunter)` markers for the date and address.

Test cards for the sandbox checkout are in Paddle's docs (`4242 4242 4242 4242`, any future expiry).

## Supabase

Create `smidge-dev` (and later `smidge-prod`). Apply `supabase/migrations/0001_init.sql` then `0002_functions.sql` in the SQL editor (or `supabase db push` after `supabase link`). Put the project URL and the **service role** key in the env; the anon key is never used. RLS is on with no policies, so nothing but the service role can read.

After applying migrations, regenerate the dump: `supabase db dump --schema-only > apps/site/supabase/schema.sql`. CI's `site` job compares the two.

## Resend

Verify `mail.<domain>` (SPF and DKIM records Resend gives you), create an API key, set `RESEND_API_KEY`, `EMAIL_FROM` (`Smidge <hello@mail.<domain>>`) and `EMAIL_REPLY_TO`.

## Deploy

Vercel project `smidge-site`, root directory `apps/site`, framework Next.js. `vercel.json` sets the two crons:

| Path | Schedule | Does |
|---|---|---|
| `/api/v1/cron/reprocess-webhooks` | every 10 minutes | retries `error` events and `received` events older than 2 minutes, up to 20 attempts, then emails `ADMIN_ALERT_EMAIL` once per event |
| `/api/v1/cron/housekeeping` | hourly | expires claims, deletes used/expired OTP rows, deletes rate-limit windows older than a day, idles desktop devices unseen for 180 days |

Vercel sends `Authorization: Bearer $CRON_SECRET` automatically when `CRON_SECRET` is set in the project.

Environment variables (all of DESIGN.md 5.13, see `.env.example`): `NEXT_PUBLIC_SITE_URL`, `NEXT_PUBLIC_APP_URL`, `SUPABASE_URL`, `SUPABASE_SERVICE_ROLE_KEY`, `PADDLE_ENV`, `PADDLE_API_KEY`, `PADDLE_WEBHOOK_SECRET`, `PADDLE_PRICE_ID_LIFETIME`, `PADDLE_PRICE_ID_YEARLY`, `NEXT_PUBLIC_PADDLE_ENV`, `NEXT_PUBLIC_PADDLE_CLIENT_TOKEN`, `RESEND_API_KEY`, `EMAIL_FROM`, `EMAIL_REPLY_TO`, `LICENSE_SIGNING_KEY`, `LICENSE_SIGNING_KID`, `KEY_ENCRYPTION_KEY`, `OTP_PEPPER`, `IP_HASH_SALT`, `CRON_SECRET`, `ADMIN_ALERT_EMAIL`. Optional: `SMG_DB=memory` (never in production; env validation refuses it with `PADDLE_ENV=production`).

Deploys go through the Vercel CLI in `site.yml` (previews on PRs, production behind the `production` GitHub environment), never Git-triggered deploys, as DESIGN.md 6.5 says.

## Notes that differ from DESIGN.md

- The `smg_chk` cookie is set with `Path=/api/v1/checkout`, not `Path=/success`: the cookie must reach `GET /api/v1/checkout/{id}`, which a `/success` path would never send it to. The browser still gets to `/success?c=chk_...` through Paddle's `successUrl`.
- `POST /checkout` answers `{transaction_id, checkout_id}`; the extra `checkout_id` is what the success URL carries. `CheckoutCreateResponseSchema` strips unknown keys, so clients that parse with it are unaffected.
- `GET /account` adds `revoked_at` to each entitlement for the dashboard's "Refunded on" line, for the same reason.
- Email templates are plain HTML strings in `emails/*.tsx` rather than React Email components, to keep the dependency list short; `sendOnce` and the `email_log` unique key are as specified.
- Origin check: an `Origin` header, when present, must be the site, the app, or Tauri's webview origin (`tauri://localhost`, `http://tauri.localhost`). Site-only POSTs (`/checkout`, `/auth/*`, `/account/*`) require the header; app routes accept its absence because the desktop app's HTTP client sends none.
