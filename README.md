# kitsunebi

kitsunebi is an operational foundation for MCPlayNetwork's long-running
Minecraft network. It is not a general game-server hosting panel, a
Pterodactyl-compatible UI, or a Docker-only Minecraft template.

The implementation is organized around a CLI and a Rust core library:

- `kitsunebi status <target>`
- `kitsunebi start|stop|restart <target>`
- `kitsunebi logs <target>`
- `kitsunebi cmd <target> -- "<game command>"`
- `kitsunebi dev up|down|reset`
- `kitsunebi plugin diff|sync <target>`
- `kitsunebi plugin lock`
- `kitsunebi plugin update-plan <plugin> --to <version>`
- `kitsunebi plugin three-way-diff <target> <path> <migrated-file>`
- `kitsunebi config diff|drift|apply <target>`
- `kitsunebi backup preflight <target>`
- `kitsunebi maintenance restart <target> --confirm`
- `kitsunebi materialize <target>`

The first production runtime adapter is `systemd-java`. Docker Compose is kept
as the standard development topology and as an optional runtime adapter, but it
is not the production design center.

## Repository Layout

```text
src/
  lib.rs              # core API, inventory loader, adapters, managers
  main.rs             # CLI entrypoint

inventory/
  production.yaml     # production target definitions
  development.yaml    # local/dev target definitions

instances/
  <instance>/
    configs/          # explicitly managed config files
    plugins/          # optional desired manual jar set for this instance
    plugin-policy.yaml
    instance.yaml

plugins/
  manual/<instance>/  # preferred manual artifact source

templates/
  systemd/
  docker-compose/

docs/
  spec.md
  operations.md
```

Live data, secrets, logs, generated files, cache, and production worlds do not
belong in Git.

## Build and Test

```bash
cargo test
cargo build
```

The implementation currently avoids external Rust dependencies so it can
build without registry access.

## Basic Usage

Use the production inventory by default:

```bash
kitsunebi status backend-vanilla-1
kitsunebi logs backend-vanilla-1 --lines 200
kitsunebi cmd backend-vanilla-1 -- "list"
```

Use a specific inventory:

```bash
kitsunebi --inventory inventory/development.yaml status
```

Start the development topology:

```bash
kitsunebi dev up
kitsunebi dev logs dev-vanilla
kitsunebi dev cmd dev-vanilla -- "list"
kitsunebi dev down
```

Manage manual plugin artifacts:

```bash
kitsunebi plugin diff backend-vanilla-1
kitsunebi plugin sync backend-vanilla-1
kitsunebi plugin lock
kitsunebi plugin update-plan luckperms --to 5.4.152
kitsunebi plugin three-way-diff backend-vanilla-1 plugins/LuckPerms/config.yml /tmp/migrated-config.yml
```

Manage explicit config files:

```bash
kitsunebi config diff backend-vanilla-1
kitsunebi config apply backend-vanilla-1
kitsunebi config import backend-vanilla-1 plugins/LuckPerms/config.yml
```

`config apply` refuses to overwrite live drift or conflicts unless
`--overwrite-conflicts` is explicitly passed.

Production workflow helpers:

```bash
kitsunebi backup preflight backend-vanilla-1
kitsunebi maintenance restart backend-vanilla-1 --notice "maintenance restart" --confirm
kitsunebi materialize backend-vanilla-1
```

## Secret Boundary

RCON passwords are read from one of these locations:

- `KITSUNEBI_RCON_PASSWORD`
- `KITSUNEBI_<TARGET>_RCON_PASSWORD`
- `/etc/kitsunebi/secrets/<target>.env`
- `secrets/<target>.env`
- `secrets/rcon.env`

Each env file uses `RCON_PASSWORD=...`. Secrets must not be committed.
