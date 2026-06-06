# Architecture

Paschal is a single-binary application that embeds the web UI and serves both the API and the SvelteKit frontend from one process. PostgreSQL holds all state; a local file-backed key encrypts attachment blobs.

---

## System overview

```mermaid
graph TD
    subgraph Client
        Browser["Browser (SvelteKit SPA)"]
        CLI["paschal-cli"]
        iCloud["Apple iCloud Shortcut"]
        Bank["Bank dormancy webhook"]
        MS["Microsoft Account ping"]
    end

    subgraph "Beacon (single binary)"
        API["Axum HTTP server"]
        Scheduler["Signal aggregator\n(background task)"]
        SPA["Embedded SvelteKit assets"]
        Migrator["SQL migrations\n(run on boot)"]
    end

    subgraph Storage
        PG[(PostgreSQL 16)]
        Blobs["Local filesystem\n(encrypted blobs + kms.key)"]
    end

    Browser -->|"REST /v1/*"| API
    CLI -->|"REST /v1/*"| API
    iCloud -->|"POST /v1/signals/apple-icloud/ping"| API
    Bank -->|"POST /v1/signals/bank-dormancy/:id"| API
    MS -->|"POST /v1/signals/microsoft/observe"| API

    API --> PG
    API --> Blobs
    Scheduler --> PG
    SPA --> Browser
    Migrator --> PG
```

---

## Vault state machine

Vaults move through a deterministic state machine. The signal aggregator drives most transitions; the principal can manually force a release or cancel cooling-off.

```mermaid
stateDiagram-v2
    [*] --> ACTIVE : vault created

    ACTIVE --> SUSPICIOUS : aggregator: score rising
    SUSPICIOUS --> ACTIVE : aggregator: score fell / heartbeat
    SUSPICIOUS --> ALERT : aggregator: score escalated

    ALERT --> SUSPICIOUS : aggregator: score fell
    ALERT --> COOLING_OFF : aggregator: threshold crossed\nor force-release

    ACTIVE --> COOLING_OFF : force-release (principal or co-steward)

    COOLING_OFF --> ACTIVE : principal cancels\ncooling-off window
    COOLING_OFF --> RELEASING : cooling-off window expires

    RELEASING --> RELEASED : scheduler: all letters dispatched
    RELEASED --> ARCHIVED : retention window expires
```

---

## Release flow (signal-triggered)

```mermaid
sequenceDiagram
    participant Scheduler
    participant DB as PostgreSQL
    participant Notifier as Email notifier
    participant Recipient

    loop Every AGGREGATOR_TICK_SECONDS
        Scheduler->>DB: fetch active vaults
        Scheduler->>DB: fetch recent signals per vault
        Scheduler->>Scheduler: compute TriggerScore (L2 norm)
        alt score ≥ threshold AND ≥ min independent classes
            Scheduler->>DB: transition vault → COOLING_OFF
            Scheduler->>Notifier: send cooling-off warning to principal
        end
    end

    Note over Scheduler,DB: After COOLING_OFF_SECONDS with no cancellation

    Scheduler->>DB: transition vault → RELEASING
    Scheduler->>DB: fetch all letters for vault
    loop Each letter
        Scheduler->>DB: generate single-use claim token
        Scheduler->>Notifier: send claim email to recipient
    end
    Scheduler->>DB: transition vault → RELEASED
```

---

## Claim flow (recipient)

```mermaid
sequenceDiagram
    participant Recipient
    participant ClaimPage as /claim (heir SPA)
    participant API as Beacon API
    participant Blobs as Blob store

    Recipient->>ClaimPage: opens link from email (contains token)
    ClaimPage->>API: GET /v1/releases/claim?token=...
    API->>API: validate token, mark used
    API-->>ClaimPage: letter body (decrypted) + attachment list
    ClaimPage-->>Recipient: render letter

    opt Has attachments
        Recipient->>ClaimPage: click attachment
        ClaimPage->>API: GET /v1/releases/claim/attachment?token=...&attachment_id=...
        API->>Blobs: fetch ciphertext
        API->>API: decrypt
        API-->>ClaimPage: file stream
        ClaimPage-->>Recipient: download
    end
```

---

## Signal scoring

The aggregator scores each vault using a weighted L2 norm across independent signal sources:

```
score = clamp(sqrt(Σ (weight_i × contribution_i)²), 0, 1)
```

`contribution_i` decays exponentially with time since the signal was observed (half-life ≈ 14 days). Observations older than 60 days are ignored.

An alert fires when:
- `score ≥ threshold` (default 0.7), **and**
- at least N independent signal classes are contributing (prevents a single source from triggering alone).

```mermaid
graph LR
    subgraph Signals["Signal sources (examples)"]
        H["Heartbeat (weight 0.30)"]
        B["Buddy attestation (weight 0.20)"]
        G["Guardian attestation (weight 0.40)"]
        C["CDR / bank dormancy (weight 0.25)"]
        A["Apple iCloud Shortcut (weight 0.18)"]
    end
    Signals --> Decay["Time-decay\nc(t) = c₀·exp(−t/τ)"]
    Decay --> L2["L2 aggregation\nscore = √Σ(wᵢ·cᵢ)²"]
    L2 --> Threshold{"score ≥ 0.7\nAND ≥ 2 classes?"}
    Threshold -->|yes| Alert["Vault → COOLING_OFF"]
    Threshold -->|no| Wait["Wait next tick"]
```

---

## Binary build

The Dockerfile uses a multi-stage build:

1. **node:22-slim** — runs `pnpm build`, producing the SvelteKit static output.
2. **cargo-chef planner** — fingerprints Rust dependencies.
3. **cargo-chef builder** — compiles the Rust binary with SvelteKit assets embedded via `include_dir!`.
4. **debian:bookworm-slim** — minimal runtime image; only the compiled binary, migrations directory, and heir claim page.

The result is a single container image (~60 MiB) with no Node runtime at all.
