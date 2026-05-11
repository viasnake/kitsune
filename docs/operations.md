# Operations

## Inventory Selection

By default, kitsunebi reads `inventory/production.yaml` when it exists. Use
`--inventory` or `KITSUNEBI_INVENTORY` to select another file.

```bash
kitsunebi --inventory inventory/development.yaml status
KITSUNEBI_INVENTORY=inventory/development.yaml kitsunebi status
```

## Production Status and Logs

Production instances are expected to use systemd units named
`kitsunebi@<instance>.service` unless `logs.journald_unit` overrides the unit in
inventory.

```bash
kitsunebi status backend-vanilla-1
kitsunebi restart backend-vanilla-1
kitsunebi logs backend-vanilla-1 --lines 200
kitsunebi logs backend-vanilla-1 --follow
```

## Game Commands

RCON is the only initial command transport.

```bash
kitsunebi cmd backend-vanilla-1 -- "list"
kitsunebi cmd backend-vanilla-1 -- "say maintenance starts in 5 minutes"
```

RCON password lookup order:

1. `KITSUNEBI_RCON_PASSWORD`
2. `KITSUNEBI_<TARGET>_RCON_PASSWORD`
3. `/etc/kitsunebi/secrets/<target>.env`
4. `secrets/<target>.env`
5. `secrets/rcon.env`

The env file format is:

```env
RCON_PASSWORD=change-me
```

## Development Topology

The default development compose file is
`templates/docker-compose/dev-stack.yml`. Override it with
`KITSUNEBI_DEV_COMPOSE`.

```bash
kitsunebi dev up
kitsunebi dev logs dev-vanilla
kitsunebi dev cmd dev-vanilla -- "list"
kitsunebi dev reset
```

`dev reset` removes compose volumes for the dev topology.

## Plugin Diff and Sync

The manual artifact workflow searches desired jars in this order:

1. `instances/<target>/plugins/`
2. `plugins/manual/<target>/`
3. `plugins/dist/<target>/`

```bash
kitsunebi plugin diff backend-vanilla-1
kitsunebi plugin sync backend-vanilla-1
kitsunebi plugin lock
kitsunebi plugin update-plan luckperms --to 5.4.152
kitsunebi plugin three-way-diff backend-vanilla-1 plugins/LuckPerms/config.yml /tmp/migrated-config.yml
```

Unknown jars in the live tree are never deleted by these commands.

## Config Diff, Apply, and Import

Managed config is explicit. Put files under `instances/<target>/configs/` using
the same relative path they should have below the live tree.

```bash
kitsunebi config diff backend-vanilla-1
kitsunebi config apply backend-vanilla-1
kitsunebi config import backend-vanilla-1 plugins/LuckPerms/config.yml
```

`config apply` snapshots overwritten live files before copying and writes
`runtime/last-applied.json` under the instance root. It refuses to overwrite
live drift or conflicts unless `--overwrite-conflicts` is passed.

## Materialize

`materialize` is the production preparation workflow for one instance.

```bash
kitsunebi materialize backend-vanilla-1
```

It loads inventory, resolves the target, reports plugin and config differences,
syncs manual plugin artifacts, applies managed config, and prints runtime
status. It keeps the same safety behavior as `config apply`.

## Backup and Maintenance

Backup storage and DB snapshots are external, but kitsunebi provides a preflight
view over the instance paths and plugin policy.

```bash
kitsunebi backup preflight backend-vanilla-1
```

Maintenance restart is an explicit workflow and requires confirmation.

```bash
kitsunebi maintenance restart backend-vanilla-1 --notice "maintenance restart" --confirm
```
