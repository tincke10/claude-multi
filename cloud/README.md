# claude-multi cloud (session tracking backend)

Optional backend for the **Tracking** feature of claude-multi. You deploy it to
your own Cloudflare account; nothing here is hosted for you.

## Architecture

```
claude ─metrics (OTLP http/protobuf)──────────────► Grafana Cloud
claude ─logs (OTLP http/json, Bearer)─► cm-ingest ──► D1 (cm-db) ◄── cm-dashboard ◄── browser
hooks  ─► cms --hook ─► cm-ingest /api/events (+ IP / geo) ┘          (Cloudflare Access)
            └─ detached `cms --summarize` ─► claude -p (Haiku) ─► /api/events {type: summary}
```

| Piece | What it is | What it stores / does |
|---|---|---|
| `cm-ingest` (`cloud/ingest`) | Worker, **write** side. Routes: `POST /v1/logs`, `POST /api/events`, `GET /api/health` (all require `Authorization: Bearer <INGEST_TOKEN>`; anything else is 404). A daily cron (`17 3 * * *`) purges dedup rows older than 7 days | Parses Claude Code log events into per-session aggregates (counts, tokens, cost, per-tool and per-model usage) and UPSERTs them into D1. Adds public IP and geo (country/city/region/ASN) from Cloudflare request metadata to hook events. Identical log batches are deduplicated by SHA-256 so retries never double count. Optionally forwards the raw log batch to Grafana (see below) |
| `cm-dashboard` (`cloud/dashboard`) | Worker + React SPA, **read-only**. Serves the UI and `/api/sessions`, `/api/sessions/:id`, `/api/totals`, `/api/facets` | Only `SELECT`s from D1. Holds **no** ingest token and exposes no ingest route. Every request is checked for a valid Cloudflare Access JWT (`Cf-Access-Jwt-Assertion`: signature via JWKS, issuer, audience); empty `ACCESS_*` vars fail closed with 500 |
| D1 `cm-db` | One database shared by both Workers (binding `DB`) | Tables `sessions`, `session_tools`, `session_models`, `seen_batches`. Schema in `cloud/migrations/0001_init.sql`, owned and applied from `cloud/ingest` only |
| Grafana Cloud | Metrics sink (optional) | Receives Claude Code metrics **directly from the client** over OTLP; it never passes through your Workers |

What is **not** stored: raw OTLP events and prompt/response text. The ingest
parser only extracts the fields above and drops everything else. Session
summaries (if enabled) are stored as text in `sessions.summary`. Sessions are
kept indefinitely (only `seen_batches` is purged). A session with no
`SessionEnd` and no events for `STALE_AFTER_HOURS` (default 6) is *shown* as
`stale`; this is computed at read time and nothing is rewritten.

Requirements: Node.js 18+ and a Cloudflare account. Claude Code **>= 2.1.212**
on the machine running the sessions.

## Setup runbook (you do this; the repo never does it for you)

All commands run from the repo root unless a `cd` is shown. Wrangler is a dev
dependency of the workspace, so `npx wrangler` resolves to the local copy.

### 1. Install

```sh
cd cloud
npm ci
```

### 2. Log in to Cloudflare

```sh
cd cloud/ingest
npx wrangler login
```

### 3. Create the D1 database and paste its id

```sh
cd cloud/ingest
npx wrangler d1 create cm-db
```

Copy the `database_id` it prints into **both** files, replacing the
`00000000-0000-0000-0000-000000000000` placeholder:

- `cloud/ingest/wrangler.jsonc` (`d1_databases[0].database_id`)
- `cloud/dashboard/wrangler.jsonc` (`d1_databases[0].database_id`)

Keep `database_name` as `cm-db` and the binding as `DB`.

### 4. Apply the migrations (remote, from ingest only)

```sh
cd cloud/ingest
npx wrangler d1 migrations apply cm-db --remote
```

Only `cm-ingest` references the migrations directory
(`migrations_dir: "../migrations"`). Do not run this from `cloud/dashboard`.

### 5. Set the secrets (ingest)

Generate the token and keep it somewhere safe; you will paste it into
Preferences later:

```sh
openssl rand -hex 32
```

```sh
cd cloud/ingest
npx wrangler secret put INGEST_TOKEN        # paste the token when prompted
```

Optional: forward raw log batches to Grafana Cloud Logs.

```sh
npx wrangler secret put GRAFANA_AUTH
```

`GRAFANA_AUTH` is sent verbatim as the HTTP `Authorization` header value, so it
must be the **full header value**, e.g. `Basic <base64(instanceId:token)>`
(`printf '%s' '<instanceId>:<token>' | base64`). Forwarding is active only when
`GRAFANA_AUTH` **and** the `GRAFANA_OTLP_LOGS_URL` var in
`cloud/ingest/wrangler.jsonc` (empty by default) are both set; the URL is POSTed
as-is with a JSON body, so use the full OTLP logs URL
(`https://otlp-gateway-<zone>.grafana.net/otlp/v1/logs`). Forward failures are
logged and never affect the response to Claude Code. Skip this whole step if you
only want metrics in Grafana.

### 6. Deploy both Workers

```sh
cd cloud/ingest
npx wrangler deploy                 # note the printed https://cm-ingest.<subdomain>.workers.dev

cd ../dashboard
npm run build                       # vite build -> dist/client (must exist before deploy)
npx wrangler deploy                 # note https://cm-dashboard.<subdomain>.workers.dev
```

### 7. Protect the dashboard with Cloudflare Access

Until this step the dashboard fails closed (every request 500s because
`ACCESS_TEAM_DOMAIN`/`ACCESS_AUD` are empty), so nothing is exposed meanwhile.

1. Zero Trust dashboard -> **Access** -> **Applications** -> **Add an application**
   -> **Self-hosted**.
2. Application domain: the `cm-dashboard.<subdomain>.workers.dev` hostname.
   Policy: *Allow* for your own email(s) only.
3. Also cover the preview/version URLs: Workers also answer on
   `<version>-cm-dashboard.<subdomain>.workers.dev`. Either add those hostnames
   to the Access application, or enable the Access toggle in the Worker's
   *Settings -> Domains & Routes* (this toggle protecting both the `workers.dev`
   and preview URLs is from Cloudflare's docs and **was not verified here**).
   Defence in depth that **is** in the code: the Worker itself rejects any
   request without a valid Access JWT (403), so an unprotected hostname returns
   403 instead of data, but it is still worth gating it at the edge.
4. Copy the application's **Audience (AUD) tag** and your **team domain**
   (`https://<team>.cloudflareaccess.com`, with scheme, no trailing slash).
5. Put them in `cloud/dashboard/wrangler.jsonc` `vars` and redeploy:

   ```jsonc
   "ACCESS_TEAM_DOMAIN": "https://<team>.cloudflareaccess.com",
   "ACCESS_AUD": "<aud tag>",
   ```

   ```sh
   cd cloud/dashboard && npm run build && npx wrangler deploy
   ```

### 8. Grafana Cloud (metrics)

1. Create a Grafana Cloud stack (free tier is enough).
2. In the stack's **OpenTelemetry** / OTLP connection page note the **OTLP
   endpoint** (`https://otlp-gateway-<zone>.grafana.net/otlp`) and the
   **Instance ID**.
3. Create an access-policy token for it. Scope `metrics:write` is what OTLP
   metrics ingestion needs; `logs:write` only matters if you use the optional
   `GRAFANA_AUTH` forwarding. Both scope names are from memory of Grafana's docs
   and **not verified here**. Tracking sends metrics only; the client never
   needs read scopes.

### 9. Configure claude-multi

Preferences -> **Tracking**:

| Field | Value |
|---|---|
| Ingest URL | `https://cm-ingest.<subdomain>.workers.dev` (base URL, no path) |
| Ingest token | the `INGEST_TOKEN` from step 5 |
| OTLP endpoint / Instance ID / Access token | from step 8 (all three or none) |
| AI summary | optional; see the cost caveat in the root README |

Click **Test connection** (works before enabling). Ingest: `GET /api/health`
with the bearer token (200 = ok, 401 = token rejected, 404 = wrong URL).
Grafana: a best-effort empty metrics POST; 2xx/400/415/422 count as "credentials
accepted", 401/403 as rejected. It cannot prove metrics will render. It also
warns if the `claude` found in `~/.local/bin`, `/opt/homebrew/bin` or
`/usr/local/bin` is older than 2.1.212 (no warning for other install
locations). Then tick **Enabled** and **Save**.

## Manual verification checklist

These need a real Claude Code, so they are **not** covered by automated tests.
Tick them once after the first deploy.

- [ ] **Real OTLP payload / `TOKEN_ATTR`.** The token attribute names in
  `cloud/packages/core/src/types.ts` (`TOKEN_ATTR`: `input_tokens`,
  `output_tokens`, `cache_read_tokens`, `cache_creation_tokens`) are
  **unconfirmed** against Claude Code. Capture a payload and compare:
  - Locally: `CLAUDE_CODE_ENABLE_TELEMETRY=1 OTEL_LOGS_EXPORTER=console claude`
    and send a prompt; look at the `claude_code.api_request` record attributes.
  - `wrangler tail` is **not** enough: the Worker logs counts (`logs_ingested`),
    never payloads, on purpose.
  - If a name differs, fix only `TOKEN_ATTR` (one-line change) and add a fixture.
  - Symptom of a wrong name: sessions appear with prompts/requests but 0 tokens.
- [ ] **Hook stdin field.** The SessionEnd cause is parsed as `reason` with `why`
  accepted as an alias (`src-tauri/src/hook.rs`). Confirm which one your Claude
  Code sends; the session detail should show an end reason, not blank.
- [ ] **Detached summary survives terminal close.** With AI summary on, end a
  session and immediately close the terminal window; the summary should still
  arrive (spawned with its own process group on Unix, `DETACHED_PROCESS` on
  Windows; unverified in practice).
- [ ] **`--resume` from another cwd.** The summary worker runs
  `claude -p --resume <transcript_path> --model haiku ...` from the session's
  `cwd`. Confirm it works when that directory no longer exists or differs (the
  worker falls back to its own cwd if `cwd` is not a directory).
- [ ] **Summary cost.** After a few sessions, check the account's usage: each
  summary is one Haiku call over the whole transcript (no token cap, 300 s
  timeout, `--max-turns 1`) billed to the **same account** as the session.
- [ ] **Metrics in Grafana.** Explore -> your Prometheus datasource; Claude Code
  metrics (names are Claude Code's, e.g. `claude_code_*`) appear within a minute
  or two of starting a session.

## Free plan limits and watching CPU

Figures below were checked on 2026-10-09 against Cloudflare's published pages
([Workers limits](https://developers.cloudflare.com/workers/platform/limits/),
[D1 limits](https://developers.cloudflare.com/d1/platform/limits/),
[D1 pricing](https://developers.cloudflare.com/d1/platform/pricing/)). Limits
change, so re-check those pages before relying on them:

- Workers Free: 100,000 requests/day and 10 ms CPU per invocation.
- D1 Free: 5 M rows read/day, 100 k rows written/day, 500 MB per database,
  5 GB total storage per account.

Load profile: with tracking on, each running session posts a log batch roughly
every 5 s (`OTEL_LOGS_EXPORT_INTERVAL=5000`, set by claude-multi) while it has
events, and each *new* batch costs one `seen_batches` insert plus session, tool
and model upserts (writes). Hooks add 2 requests per session (3 with a summary). One person with a
few parallel sessions should stay well inside the limits; many busy sessions
all day may not.

Watching it:

- Cloudflare dashboard -> Workers & Pages -> `cm-ingest` -> **Metrics**
  (requests, errors, **CPU time**) and D1 -> `cm-db` -> **Metrics** (rows
  read/written).
- Live: `cd cloud/ingest && npx wrangler tail` (structured JSON logs, never the
  token). Errors from the DB return 503 + `Retry-After: 5`, which is the retryable signal
  for OTLP exporters (Claude Code's retry behaviour is unverified); dedup makes a
  retry safe.
- Both Workers have `observability` enabled with sampling rate 1.

## Developing and testing

```sh
cd cloud
npm test            # vitest in packages/*, ingest, dashboard (Workers pool + local D1)
npm run typecheck   # wrangler types + tsc for every workspace
```

`npm run dev` in `ingest` (`wrangler dev`) or `dashboard`
(`vite build && wrangler dev`) runs a Worker locally. Do not point a local run
at the remote D1 by accident; `wrangler dev` uses a local D1 unless told
otherwise.
