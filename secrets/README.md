# secrets

This directory is for local development placeholders only. Production secrets
belong under `/etc/kitsunebi/secrets/` and must not be committed.

RCON command execution reads `RCON_PASSWORD=...` from:

- `/etc/kitsunebi/secrets/<target>.env`
- `secrets/<target>.env`
- `secrets/rcon.env`

Database, Redis, backup, and external service credentials are outside the
kitsunebi Git boundary.
