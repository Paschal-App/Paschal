[![CD](https://github.com/Paschal-App/Paschal/actions/workflows/cd.yml/badge.svg)](https://github.com/Paschal-App/Paschal/actions/workflows/cd.yml)
[![Dependabot Updates](https://github.com/Paschal-App/Paschal/actions/workflows/dependabot/dependabot-updates/badge.svg)](https://github.com/Paschal-App/Paschal/actions/workflows/dependabot/dependabot-updates)
🔥✉️# Paschal

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

## Managed hosting

If you would rather not run your own server, **[paschal.app](https://paschal.app)**
offers a fully managed, hosted version of Paschal — backups, upgrades, and
monitoring included. The self-hosted and managed editions share the same
open-source core.

---

## What it does

- **Vaults and letters.** Author letters — text, files, or credential bundles —
  and seal them. Each vault names who receives it and on what terms.
- **Check-ins.** While you are active, periodic check-ins keep every vault shut.
- **The switch.** When check-ins lapse beyond your configured window, the system
  begins to trip. A **cooling-off period** gives you (or a trusted co-steward) time
  to cancel a false alarm before anything is released.
- **Release.** After cooling-off, recipients are notified and can claim what you
  left them through a dedicated claim page.
- **Encryption at rest.** Letter contents and attachments are encrypted with
  AES-256-GCM. The encryption key is held locally on the instance you run.
- **Signal sources.** Beyond manual check-ins, the system can watch Apple
  iCloud Shortcuts, Microsoft Account logins, bank dormancy webhooks, buddy
  attestations, and more to keep vaults alive.
- **Drills.** Run a rehearsal release any time to verify the full flow without
  committing — vaults return to ACTIVE after a drill.

## How it is built

| Layer | Technology |
|---|---|
| Backend | Rust · Axum · SQLx · Tokio |
| Database | PostgreSQL 16 |
| Frontend | SvelteKit 2 · Svelte 5 · TypeScript (compiled into the binary) |
| Encryption | AES-256-GCM, local file-backed key |

The SvelteKit app is compiled into the Rust binary at build time, so the running
container serves both the API and the UI from one process. Database migrations
are embedded and apply automatically on first boot.

See [`docs/architecture.md`](docs/architecture.md) for system diagrams.

---

## Quickstart

You need [Docker](https://docs.docker.com/get-docker/) with Compose.

```bash
git clone https://github.com/yourusername/pascal-os.git paschal && cd paschal
cp .env.example .env
# edit .env — at minimum set POSTGRES_PASSWORD to a long random string
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
| `RETENTION_DAYS` | `1095` (3 years) | Data retention after subscription cancellation |
| `MAX_UPLOAD_BYTES` | `16777216` (16 MiB) | Maximum single file upload size |

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

---

## Documentation

| Document | Contents |
|---|---|
| [`docs/how-it-works.md`](docs/how-it-works.md) | How the dead-man's switch works end-to-end |
| [`docs/architecture.md`](docs/architecture.md) | System components and Mermaid diagrams |
| [`docs/self-hosting.md`](docs/self-hosting.md) | Detailed self-hosting and operations guide |
| [`docs/api.md`](docs/api.md) | HTTP API reference |

---

## Repository layout

```
crates/
  beacon-api/           HTTP server, scheduler, signal aggregation
  beacon-core/          domain logic — state machines, signal scoring
  beacon-db/            database queries and embedded migrations
  crypto-stub/          AES-256-GCM key abstraction (local file key)
  paschal-transform/    attachment pipeline (re-encodes to archive-safe formats)
  paschal-cli/          operator command-line tool
  blob-store/           attachment storage abstraction (local filesystem)
apps/
  principal/            the SvelteKit web app
  heir/                 the static recipient claim page
migrations/             sequential SQL migrations
docs/                   architecture and operations documentation
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
