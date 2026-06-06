# Self-hosting guide

This guide covers everything you need to run Paschal reliably on your own server.

---

## Prerequisites

- A Linux server (any major distro).
- [Docker Engine](https://docs.docker.com/engine/install/) with the Compose plugin.
- A domain name with DNS pointed at the server (for TLS and correct recipient links).
- An outbound SMTP relay if you want email notifications (SendGrid, Postmark, SES, or any SMTP server).

---

## Quick setup

```bash
git clone https://github.com/yourusername/pascal-os.git paschal
cd paschal
cp .env.example .env
```

Edit `.env`:

```env
POSTGRES_PASSWORD=<long-random-string>
BEACON_PUBLIC_BASE_URL=https://vault.example.com
```

Then:

```bash
docker compose up -d --build
```

The first build compiles the Rust binary and the frontend (~5–10 minutes). Subsequent starts are fast (the image is cached).

---

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `POSTGRES_USER` | `paschal` | Database user |
| `POSTGRES_PASSWORD` | — **(required)** | Database password — use a long random string |
| `POSTGRES_DB` | `paschal` | Database name |
| `BEACON_PUBLIC_BASE_URL` | `http://localhost:8080` | The external URL principals and recipients use. Must be set correctly for email links to work. |
| `COOLING_OFF_SECONDS` | `1209600` | Cooling-off window in seconds (default: 14 days). |
| `HEARTBEAT_MAX_GAP_SECONDS` | `604800` | Maximum gap between check-ins before the switch starts tripping (default: 7 days). |
| `AGGREGATOR_TICK_SECONDS` | `300` | How often the signal aggregator re-evaluates vaults (default: 5 minutes). |
| `TRIAL_DAYS` | `30` | Trial period length in days. Informational only in self-hosted mode. |
| `RETENTION_DAYS` | `1095` | How long data is retained after account cancellation (default: 3 years). |
| `MAX_UPLOAD_BYTES` | `16777216` | Maximum individual file upload size in bytes (default: 16 MiB). |
| `RATE_LIMIT_PER_MINUTE` | `240` | Request rate limit per IP per minute. |
| `SMTP_HOST` | — | SMTP server hostname. If unset, emails are logged to stdout only. |
| `SMTP_PORT` | `587` | SMTP port (587 for STARTTLS, 465 for implicit TLS). |
| `SMTP_USERNAME` | — | SMTP authentication username. |
| `SMTP_PASSWORD` | — | SMTP authentication password. |
| `SMTP_FROM` | — | From address for outbound emails. |

### AWS KMS (optional)

By default, the encryption key is stored as a local file (`kms.key`) in the `beacon_data` volume. For higher security, you can use AWS KMS:

| Variable | Description |
|---|---|
| `KMS_BACKEND` | Set to `aws` to use AWS KMS. Default: `local`. |
| `KMS_KEY_ARN` | ARN of the AWS KMS CMK to use for wrapping the data encryption key. |
| `AWS_REGION` | AWS region for the KMS key. |

### AWS S3 blob storage (optional)

Attachment blobs are stored on the local filesystem by default. For multi-region or cloud deployments:

| Variable | Description |
|---|---|
| `BLOB_BACKEND` | Set to `s3` to use S3. Default: `local`. |
| `BLOB_S3_BUCKET` | S3 bucket name for blob storage. |

---

## TLS with Caddy

The easiest way to add HTTPS is with [Caddy](https://caddyserver.com/), which handles certificate provisioning automatically:

```caddy
# /etc/caddy/Caddyfile
vault.example.com {
    reverse_proxy beacon:8080
}
```

Run Caddy on the host or add it to `docker-compose.yml`:

```yaml
services:
  caddy:
    image: caddy:2
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile
      - caddy_data:/data
    depends_on:
      - beacon

volumes:
  caddy_data:
```

Then set `BEACON_PUBLIC_BASE_URL=https://vault.example.com` and rebuild.

---

## Backups

### What to back up

Two Docker volumes hold everything that cannot be regenerated:

| Volume | Contents | Backup priority |
|---|---|---|
| `beacon_data` | Encrypted attachment blobs **and `kms.key`** | **Critical** |
| `postgres_data` | All vault metadata, letters, signals, transparency log | **Critical** |

**A backup of one without the other is not enough.** The key and the database are both required to recover encrypted content.

### Backup procedure

```bash
# Stop the stack (for a consistent snapshot)
docker compose stop beacon

# Export Postgres
docker compose exec postgres pg_dump -U paschal paschal > backup_$(date +%Y%m%d).sql

# Copy the beacon data volume
docker run --rm \
  -v pascal-os_beacon_data:/data \
  -v $(pwd):/backup \
  alpine tar czf /backup/beacon_data_$(date +%Y%m%d).tar.gz /data

docker compose start beacon
```

Store backups off-site (cloud storage, a different machine, or an encrypted drive in a different physical location).

### Restore

```bash
# Restore the database
docker compose exec -T postgres psql -U paschal paschal < backup_20240101.sql

# Restore the beacon data volume
docker run --rm \
  -v pascal-os_beacon_data:/data \
  -v $(pwd):/backup \
  alpine tar xzf /backup/beacon_data_20240101.tar.gz -C /
```

---

## Upgrades

```bash
git pull
docker compose build
docker compose up -d
```

Migrations run automatically on start. They are additive only (no destructive schema changes); rolling back to the previous image after a migration has run may leave unknown columns in the database, which Paschal handles gracefully.

Always back up before upgrading.

---

## Monitoring

The application exposes three endpoints suitable for health checks and monitoring:

| Endpoint | Returns | Notes |
|---|---|---|
| `GET /livez` | `200 OK` | Process is running. Use for container health checks. |
| `GET /readyz` | `200 OK` or `503` | Returns 503 if the database is unreachable. |
| `GET /metrics` | Prometheus text | Signups, vault count, heartbeat count, release counts, etc. |

Example `docker-compose.yml` health check (already included in the default config):

```yaml
healthcheck:
  test: ["CMD", "curl", "-f", "http://localhost:8080/livez"]
  interval: 30s
  timeout: 5s
  retries: 3
  start_period: 30s
```

---

## Resource requirements

The default `docker-compose.yml` sets:

- Memory limit: 512 MiB
- Memory reservation: 128 MiB

For a personal instance with a few principals, 256 MiB is sufficient. The signal aggregator background task is the heaviest periodic load.

PostgreSQL typically uses 64–128 MiB for a small deployment.

---

## Security considerations

- **Do not expose port 8080 directly.** Always terminate TLS at a reverse proxy.
- **Back up `kms.key` securely.** Anyone with this file can decrypt all letter content.
- **Set `POSTGRES_PASSWORD` to a strong random value** — the default in `.env.example` is intentionally invalid.
- **Keep the Docker image up to date.** The base image (`debian:bookworm-slim`) receives regular security updates.
- For vulnerability reporting, see [`SECURITY.md`](../SECURITY.md).

---

## Building from source (without Docker)

You need Rust 1.78+, Node 22, pnpm, and a running PostgreSQL instance.

```bash
# Build the web app (required before the server, which embeds it)
cd apps/principal
pnpm install
pnpm build
cd ../..

# Set the database URL
export DATABASE_URL=postgres://paschal:yourpassword@localhost:5432/paschal

# Run in dev mode (applies migrations on start)
cargo run --bin beacon

# Or build a release binary
cargo build --release --bin beacon
./target/release/beacon
```

The `beacon` binary applies migrations automatically on first run.
