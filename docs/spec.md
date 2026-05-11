# kitsunebi Design Specification

## Purpose

kitsunebi is the operations foundation for MCPlayNetwork's persistent Minecraft
network. It standardizes instance lifecycle operations, log access, game command
execution, development-environment reproduction, plugin artifact handling,
managed configuration, and operation-event logging.

kitsunebi is not:

- a general-purpose game-server hosting panel
- a Pterodactyl-compatible UI
- a web console implementation
- a Docker-only production architecture
- a Kubernetes orchestrator
- a database, Redis, backup, or AI runtime manager
- a DSL for Minecraft plugins

Backward compatibility with the previous kitsunebi structure is not required.
Legacy compose files and scripts are reference material only.

## Design Decisions

- The project name remains `kitsunebi`.
- Production should prefer the `systemd-java` runtime adapter.
- Docker Compose is the standard development topology and an optional runtime
  adapter, not the production center.
- External DB and Redis services are outside the kitsunebi management boundary.
- The public interface is the CLI. No HTTP API is included.
- CLI code is UI. Operational logic belongs in the Rust core library.
- Operational UX is split into status, logs, and arbitrary game command
  execution instead of a web console.
- Dedicated wrappers such as `players`, `broadcast`, `op`, and `whitelist` are
  intentionally excluded from the initial surface.
- Minecraft stdout/stderr and kitsunebi operation events go to journald.
- Vector is expected to collect journald logs; kitsunebi does not provide a log
  aggregation stack.
- Plugin jars are managed through manifest/lock/artifact-cache concepts. The
  implementation handles manual artifact diff/sync/lock without automatic
  downloading.
- Plugin config is managed by explicit file ownership. Whole plugin-directory
  synchronization is forbidden.
- Live data is protected by backup, not by Git.

## Runtime Model

```text
kitsunebi CLI
  |
  v
kitsunebi core API
  |
  +-- inventory loader
  +-- target resolver
  +-- runtime adapter
  +-- RCON command sender
  +-- log reader
  +-- plugin manager
  +-- config manager
  +-- journald event logger
  +-- dev environment manager
```

Initial runtime adapters:

- `systemd-java`
- `docker-compose` for development
- `pterodactyl-legacy` as a future migration adapter if required

## Standard Host Layout

```text
/srv/kitsunebi/
  instances/
    <instance>/
      data/
      runtime/
        last-applied.json
        config-snapshots/
        plugin-snapshots/
  artifacts/
    plugins/
  runtime/
    staging/
  backups/
    metadata/

/etc/kitsunebi/
  host.yaml
  secrets/
    <instance>.env

/var/lib/kitsunebi/
  state.db
  operations.db
```

Secrets are not stored below `/srv/kitsunebi` and must not be committed.

## Inventory

Inventory identifies operational targets. It is not treated as absolute truth;
it should be compared against the real environment.

Example:

```yaml
nodes:
  - name: kng01-game-01
    address: 10.10.30.20
    default_runtime: systemd-java

instances:
  - name: backend-vanilla-1
    role: backend
    node: kng01-game-01
    runtime: systemd-java
    paths:
      root: /srv/kitsunebi/instances/backend-vanilla-1
      live: /srv/kitsunebi/instances/backend-vanilla-1/data
    rcon:
      enabled: true
      host: 127.0.0.1
      port: 25576
      secret_ref: backend-vanilla-1/rcon
    logs:
      journald_unit: kitsunebi@backend-vanilla-1.service
```

## CLI Surface

```bash
kitsunebi status [target]
kitsunebi start <target>
kitsunebi stop <target>
kitsunebi restart <target>
kitsunebi logs <target>
kitsunebi cmd <target> -- "<game command>"

kitsunebi dev up
kitsunebi dev down
kitsunebi dev reset
kitsunebi dev logs <target>
kitsunebi dev cmd <target> -- "<game command>"

kitsunebi plugin diff <target>
kitsunebi plugin sync <target>
kitsunebi plugin lock
kitsunebi plugin update-plan <plugin> --to <version>
kitsunebi plugin three-way-diff <target> <path> <migrated-file>

kitsunebi config diff <target>
kitsunebi config drift <target>
kitsunebi config apply <target> [--overwrite-conflicts]
kitsunebi config import <target> <path>

kitsunebi materialize <target>
kitsunebi backup preflight <target>
kitsunebi maintenance restart <target> [--notice <text>] --confirm
```

## journald Events

kitsunebi writes JSON operation events to journald through `systemd-cat` when it
is available. If `systemd-cat` is missing, the event is printed to stderr.

Command events include a hash and a masked preview. Full raw commands are not
logged by default because game commands can contain secrets.

Example event fields:

```json
{
  "event": "kitsunebi.command",
  "actor": "operator",
  "target": "backend-vanilla-1",
  "operation": "cmd",
  "runtime": "rcon",
  "command_hash": "sha256:...",
  "command_preview": "say maintenance starts in 5 minutes",
  "result": "success",
  "duration_ms": 120
}
```

Masked key prefixes:

- `password=`
- `token=`
- `secret=`
- `key=`

## Plugin Artifact Management

The target design has three layers:

- `plugins.yaml`: human-managed requirements
- `plugins.lock`: resolved artifact metadata
- artifact cache: actual jar files

The implementation supports the manual artifact path:

1. Desired jars are read from `instances/<target>/plugins/`, then
   `plugins/manual/<target>/`.
2. Live jars are read from `<live>/plugins/`.
3. `plugin diff` reports desired, live, missing, and unknown jars.
4. `plugin sync` copies desired jars to live and leaves unknown live jars
   untouched.
5. `plugin sync` snapshots overwritten live jars under
   `<root>/runtime/plugin-snapshots/<timestamp>/`.
6. `plugin lock` writes manual artifact filenames and SHA-256 hashes to
   `plugins/plugins.lock`.
7. `plugin update-plan` reports affected instances and the manual migration
   checklist. It does not download jars automatically.
8. `plugin three-way-diff` compares repo managed config, live config before
   update, and plugin-migrated config by SHA-256.

Unknown jars are report-only by default.

## Config Management

Managed config is explicit-file based:

- preferred source: `instances/<target>/configs/`
- live destination: the instance `paths.live` directory

`config apply` copies only files present under the managed source directory.
Before overwriting an existing live file, it snapshots the previous live file
under `<root>/runtime/config-snapshots/<timestamp>/`. It also writes
`<root>/runtime/last-applied.json`.

`config drift` uses `last-applied.json` to classify managed files as
`missing-live`, `unchanged`, `repo-changed`, `live-drift`, `conflict`, or
`untracked-live`.

`config apply` must not delete unknown files, overwrite state by directory sync,
or commit generated plugin data. It refuses to overwrite `live-drift`,
`conflict`, or `untracked-live` unless `--overwrite-conflicts` is explicitly
passed.

## Safety Policy

- `unknown`: observe-only
- `state`: never overwrite
- `generated`: never commit, delete only with an explicit policy
- `managed`: apply only from explicit managed files

Dangerous operations such as world deletion, plugin state deletion, unknown file
deletion, restore, plugin major update, DB-backed plugin update, and conflict
overwrite require explicit options or workflows.

## Implementation Phases

1. Core UX: inventory, systemd status, journald logs, RCON command, operation
   events.
2. Development environment: Docker Compose up/down/reset/logs/cmd.
3. Manual plugin management: diff/sync and unknown jar reporting.
4. Config management: explicit managed file diff/apply/drift/import and
   last-applied metadata.
5. Plugin update workflow: lock resolver, update plan, snapshots, dev migration,
   three-way diff.
6. Production workflows: materialize, restart workflow, backup preflight, and
   maintenance helpers.

Implemented in this repository:

- Core UX commands are implemented.
- Docker Compose development commands are implemented.
- Manual plugin diff/sync/lock/update-plan is implemented.
- Config diff/drift/apply/import with last-applied metadata is implemented.
- `materialize` runs plugin diff, config diff, plugin sync, config apply, and
  runtime status.
- `start`, `stop`, and `restart` dispatch through the runtime adapter.
- `backup preflight` inspects live/runtime paths and policy warnings without
  managing backup storage.
- `maintenance restart` requires `--confirm`, optionally sends a RCON notice,
  and then dispatches runtime restart.

Explicitly not implemented because they are outside the current boundary:

- automatic plugin downloader
- HTTP API
- Web UI
- DB / Redis management
- Pterodactyl migration adapter
