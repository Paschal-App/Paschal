# Contributing to Paschal

Thanks for your interest in improving Paschal. This is a tool people may one day
depend on, so correctness and clarity matter more than speed.

## Development setup

You need Rust (1.78+), Node 22, pnpm, and Docker (for a local Postgres).

```bash
# start a local database
docker compose up -d postgres

# build the web app once (the server embeds it)
cd apps/principal && pnpm install && pnpm build && cd ../..

# run the server
export DATABASE_URL=postgres://paschal:paschal@localhost:5432/paschal
cargo run --bin beacon
```

## Tests

```bash
cargo test                                   # Rust unit + integration tests
cd apps/principal && npx playwright test     # end-to-end tests
```

Please run `cargo fmt`, `cargo clippy`, and the test suite before opening a pull
request.

## Pull requests

- Keep changes focused; one concern per PR.
- Describe the behaviour change and how you verified it.
- Add or update tests for anything that affects release logic, the signal
  scoring, or the cooling-off path — these are safety-critical.

## License of contributions

By submitting a contribution you agree it is licensed under the project's
**AGPL-3.0-or-later** license.
