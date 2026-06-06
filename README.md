# Paschal

> **Paschal** (PASS-kəl) — named for the great candle kept burning through a vigil. A digital letter of last resort.

A self-hosted **dead-man's switch** for your digital estate. You seal letters,
credentials, and instructions inside encrypted vaults. If you stop checking in,
a cooling-off window opens, and then the vault is released to the people you
named — the next of kin, executor, or friend who needs to know.

This is the open-source, self-hostable edition. It runs as a single Docker
stack with no cloud dependency: one Postgres container and one application
container that holds the API and the web app together.

> Paschal is provided as-is, without warranty of any kind. It is not legal,
> medical, or financial advice, and it is not a substitute for a will. Test your
> own deployment thoroughly before relying on it. See [`LICENSE`](LICENSE).

---

## What it does

- **Vaults and letters.** Author letters — text, files, or credential bundles —
  and seal them. Each vault names who receives it and on what terms.
- **Check-ins.** While you are active, periodic check-ins keep every vault shut.
- **The switch.** When check-ins lapse beyond your configured window, the system
  begins to trip. A **cooling-off period** gives you (or a trusted contact) time
  to cancel a false alarm before anything is released.
- **Release.** After cooling-off, recipients are notified and can claim what you
  left them through a dedicated claim page.
- **Encryption at rest.** Letter contents and attachments are encrypted with
  AES-256-GCM. The encryption key is held locally on the instance you run.

## How it is built

| Layer | Technology |
|---|---|
| Backend | Rust · Axum · SQLx · Tokio |
| Database | PostgreSQL 16 |
| Frontend | SvelteKit 2 · Svelte 5 · TypeScript (compiled into the binary) |
| Encryption | AES-256-GCM, local file-backed key |
| Payments | Stripe (optional, off by default) |

The SvelteKit app is compiled into the Rust binary at build time, so the running
container serves both the API and the UI from one process. Database migrations
are embedded and apply automatically on first boot.

---

## Quickstart

You need [Docker](https://docs.docker.com/get-docker/) with Compose.

```bash
git clone <your-fork-url> paschal && cd paschal
cp .env.example .env
# edit .env and set POSTGRES_PASSWORD to a long random string
docker compose up -d --build
```

The first build takes a while — it compiles the Rust binary and the web app.
When it is up:

- Web app: <http://localhost:8080/app>
- Health check: <http://localhost:8080/livez>

To follow logs or stop:

```bash
docker compose logs -f beacon
docker compose down            # stop (keeps your data volumes)
```

## Configuration

Everything is set through environment variables in `.env` (see
[`.env.example`](.env.example) for the full list). The ones you are most likely
to change:

| Variable | Default | Meaning |
|---|---|---|
| `POSTGRES_PASSWORD` | — (required) | Database password |
| `BEACON_PUBLIC_BASE_URL` | `http://localhost:8080` | External URL used in recipient links — set this to your real domain |
| `COOLING_OFF_SECONDS` | `1209600` (14 days) | Grace period before release |
| `HEARTBEAT_MAX_GAP_SECONDS` | `604800` (7 days) | Allowed gap between check-ins |
| `RETENTION_DAYS` | `1095` (3 years) | Data retention after cancellation |
| `STRIPE_SECRET_KEY` | empty | Set to enable billing; empty runs free |

## Back up your data — this matters

Two Docker volumes hold everything:

- `postgres_data` — the database.
- `beacon_data` — encrypted attachments **and `kms.key`, the encryption key**.

**If `kms.key` is lost, every encrypted letter and attachment becomes permanently
unrecoverable.** It is not stored anywhere else. Back up the `beacon_data` volume
(including `kms.key`) somewhere safe and separate from the server, and back up
`postgres_data` alongside it. A backup of one without the other is not enough.

## Putting it on the internet

Run a reverse proxy with TLS in front of the container — for example
[Caddy](https://caddyserver.com/) or nginx — terminating HTTPS and forwarding to
`beacon:8080`. Set `BEACON_PUBLIC_BASE_URL` to your public `https://` URL so the
links sent to recipients resolve correctly. Do not expose port 8080 directly to
the public internet without TLS.

## Optional: billing with Stripe

Billing is off by default and the app is fully functional without it. To enable
it, create recurring Prices in your Stripe dashboard, set `STRIPE_SECRET_KEY` and
`STRIPE_WEBHOOK_SECRET`, and point a Stripe webhook at
`https://<your-host>/v1/stripe/webhooks`.

---

## Repository layout

```
crates/
  beacon-api/           HTTP server, scheduler, billing
  beacon-core/          domain logic — state machines, signal scoring
  beacon-db/            database queries and embedded migrations
  crypto-stub/          AES-256-GCM key abstraction (local file key)
  paschal-transform/  attachment pipeline (re-encodes to archive-safe formats)
  paschal-cli/        operator command-line tool
  blob-store/           attachment storage abstraction (local filesystem)
apps/
  principal/            the SvelteKit web app
  heir/                 the static recipient claim page
migrations/             sequential SQL migrations
```

## Building from source (without Docker)

You need Rust (1.78+), Node 22, pnpm, and a Postgres instance.

```bash
# build the web app
cd apps/principal && pnpm install && pnpm build && cd ../..
# build and run the server (embeds the web app)
export DATABASE_URL=postgres://paschal:paschal@localhost:5432/paschal
cargo run --release --bin beacon
```

## Contributing

Contributions are welcome — see [`CONTRIBUTING.md`](CONTRIBUTING.md). To report a
security issue, please read [`SECURITY.md`](SECURITY.md) first and disclose
privately rather than opening a public issue.

## License

Licensed under the **GNU Affero General Public License v3.0 or later**
([`LICENSE`](LICENSE)). If you run a modified version as a network service, the
AGPL requires you to offer your users the corresponding source.

## Support the project

If Paschal is useful to you, you can support its development:

☕ **[buymeacoffee.com/paschalapp](https://buymeacoffee.com/paschalapp)**
