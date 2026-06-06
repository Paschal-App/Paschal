# Security Policy

Paschal handles material people intend to release only after they can no longer
act for themselves. We take security reports seriously.

## Reporting a vulnerability

Please **do not** open a public issue for security problems. Instead, report
privately to **security@paschal.app** (replace with your own contact if you fork
this project). Include:

- a description of the issue and its impact,
- steps to reproduce, and
- any suggested remediation.

We aim to acknowledge reports within a few days and will keep you updated as we
investigate. Please give us reasonable time to release a fix before any public
disclosure.

## Operator responsibilities

Self-hosting means you own the security of your deployment. At a minimum:

- **Protect `kms.key`.** It is the encryption key for all sealed content. Store
  backups securely and separately from the server. Anyone with this key plus the
  database can decrypt vault contents.
- **Terminate TLS** in front of the application; never expose it over plain HTTP
  on the public internet.
- **Use a strong, unique `POSTGRES_PASSWORD`** and restrict network access to the
  database to the application container.
- **Keep the host and images patched**, and rebuild periodically to pick up
  dependency updates.
