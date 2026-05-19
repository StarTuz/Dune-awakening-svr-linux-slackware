# Backup and Restore

This deployment has two backup layers:

1. Database dump/import through Funcom's `DatabaseOperation` custom resource.
2. Host-side archive under `/srv/backups/dune` containing the database dump plus
   enough Kubernetes and config metadata to reconstruct what was running.

The database is the important durable game state. The server pods and utility
pods are mostly regenerated from the `BattleGroup` custom resource and local
scripts, but the database contains character/world/farm state.

## Current Storage

| Item | Location |
|---|---|
| Host backup mount | `/srv/backups` |
| Dune backup root | `/srv/backups/dune` |
| Funcom DB dump staging | `/funcom/artifacts/database-dumps/<battlegroup>` |
| Server PVC dump import path | `<server-pv>/Saved/DatabaseDumps` |
| Local default user settings | `server/scripts/setup/config/User*.ini` |
| Deployed user settings | filebrowser pod `/srv/UserSettings/User*.ini` |

`/srv/backups` is a btrfs filesystem on `dune-vg/backups`, created by
`scripts/root-setup.sh`.

## Backup

Preferred interface — use `dune-ctl backup`:

```sh
dune-ctl backup list                    # list bundles, newest first (timestamp/age/size)
dune-ctl backup run                     # full backup: DB dump + k8s metadata + User*.ini
dune-ctl backup run --skip-db           # metadata-only (fast, ~5 seconds)
dune-ctl backup run --keep 14           # run + prune to 14 most recent
dune-ctl backup schedule                # install nightly cron at 3am, keep 14
dune-ctl backup schedule --show         # view installed schedule
dune-ctl backup schedule --remove       # remove scheduled job
```

The TUI Backups tab (`6`) shows the bundle list with live age/size and lets you
trigger a run with `r` (confirmation modal + streaming output pane).

The nightly schedule is installed in the `dune` user's crontab with a
`# dune-ctl-backup` marker so subsequent `schedule` calls are idempotent.

Underlying host wrapper (called by `dune-ctl backup run`):

```sh
~/dune-server/scripts/dune-backup.sh
```

For a known-good live system point, use:

```sh
sudo ~/dune-server/scripts/system-snapshot.sh known-good-YYYYMMDD
```

That wrapper first runs `dune-backup.sh`, captures status/security evidence
under `/srv/backups/dune/system-snapshots/<name>/`, then creates read-only btrfs
snapshots of `/` and `/srv/backups`. The logical database dump remains the clean
restore artifact for Postgres; the btrfs snapshots preserve the live host,
server package, k3s local-path data, and backup volume state.

For a lightweight resource-only point-in-time record, use:

```sh
sudo ~/dune-server/scripts/resource-snapshot.sh known-good-YYYYMMDD-resources
```

That writes host memory/swap, process, filesystem, Kubernetes pod/resource,
serverstats, `kubectl top`, VPA, and game-server memory watcher output under
`/srv/backups/dune/resource-snapshots/<name>/`. Process command lines are
redacted because the game server process arguments include service tokens and
database credentials.

It creates:

```text
/srv/backups/dune/<battlegroup>/<timestamp>/
  MANIFEST.txt
  database/<backup>.backup
  database/<backup>.backup.yaml
  k8s/*.yaml
  user-settings/deployed/UserEngine.ini
  user-settings/deployed/UserGame.ini
  user-settings/local/UserEngine.ini
  user-settings/local/UserGame.ini
```

The database dump itself is produced by:

```sh
~/dune-server/server/scripts/battlegroup.sh backup <backup-name>
```

That script creates a `DatabaseOperation` with `spec.action: dump`. The operator
runs Funcom's database utility and writes the dump to
`/funcom/artifacts/database-dumps/<battlegroup>/`.

The wrapper then copies the dump and companion metadata into `/srv/backups/dune`.

Verified on 2026-05-15:

- `~/dune-server/scripts/dune-backup.sh --skip-db` created a metadata-only
  bundle at
  `/srv/backups/dune/sh-db3533a2d5a25fb-xyyxbx/20260515-203757`.
- `~/dune-server/scripts/dune-backup.sh` created a full bundle at
  `/srv/backups/dune/sh-db3533a2d5a25fb-xyyxbx/20260515-203837`.
- The database dump operation
  `sh-db3533a2d5a25fb-xyyxbx-dump-20260515-203837` reached `Succeeded`.
- The resulting dump was about 1.1 MiB at
  `/funcom/artifacts/database-dumps/sh-db3533a2d5a25fb-xyyxbx/sh-db3533a2d5a25fb-xyyxbx-20260515-203837.backup`.

## Restore

Restore **overwrites the live database**. Always stop the battlegroup first.

### Preferred: dune-ctl backup restore

```sh
# 1. Check what's available
dune-ctl backup list

# 2. Stop the battlegroup
dune-ctl battlegroup stop

# 3. Restore (--yes required to prevent accidental invocation)
dune-ctl backup restore --yes <timestamp>
# or by full path:
dune-ctl backup restore --yes /srv/backups/dune/<battlegroup>/<timestamp>

# 4. Start and re-patch
dune-ctl battlegroup start
dune-ctl gateway-patch
dune-ctl status
```

`dune-ctl backup restore` stages the `.backup` file from the bundle into
`/funcom/artifacts/database-dumps/<battlegroup>/` then calls
`battlegroup.sh import`, which creates a `DatabaseOperation` with
`spec.action: import` and waits for the operator to report success.

### Manual restore (fallback)

If `dune-ctl` is unavailable, the underlying steps are:

1. Stop the battlegroup:

   ```sh
   ~/dune-server/server/scripts/battlegroup.sh stop
   ```

2. Stage the dump file:

   ```sh
   sudo mkdir -p /funcom/artifacts/database-dumps/<battlegroup>
   sudo cp /srv/backups/dune/<battlegroup>/<timestamp>/database/<backup>.backup \
     /funcom/artifacts/database-dumps/<battlegroup>/
   ```

3. Run the import:

   ```sh
   ~/dune-server/server/scripts/battlegroup.sh import <backup>.backup
   ```

4. Start and re-patch:

   ```sh
   ~/dune-server/server/scripts/battlegroup.sh start
   ~/dune-server/scripts/gateway-patch.sh
   dune-ctl status
   ```

## What Must Be Backed Up

Minimum viable backup:

- Database dump.
- `BattleGroup` YAML.
- `DatabaseDeployment` YAML.
- PVC/PV YAML.
- Deployed `UserSettings`.
- Local `server/scripts/setup/config/User*.ini`.

Useful extra context:

- `dune-ctl settings list`.
- `dune-ctl diagnostics`.
- `kubectl get all` for the battlegroup namespace.
- Current repo commit.

## Operator CRDs

Relevant CRDs shipped with the server package:

- `DatabaseOperation`: logical `dump` and destructive `import`.
- `DatabaseBackup`: physical backup request.
- `DatabaseBackupSchedule`: scheduled physical backup.
- `DatabaseMigrate`: physical backup restore/migration target.

For this server, the first reliable layer should be the existing
`DatabaseOperation` dump/import path. Physical backups can be investigated later
after confirming where WAL/basebackup artifacts are stored and how retention is
handled by the database operator on this single-node local-path setup.

## Open Questions

- Whether Funcom's physical `DatabaseBackup` resources write to a local artifact
  path, object storage, or a PVC in this package.
- Whether scheduled `DatabaseBackupSchedule` is worth enabling for this
  single-node host once logical dumps are proven.
- Off-host replication target: NAS via `rsync`, cloud via `rclone`, or both
  (currently backups are local-only on `dune-vg/backups`).
- Restore drill cadence. A backup should not be considered proven until an
  import has been tested on a disposable battlegroup or fresh cluster.

## Scheduling (resolved)

Nightly backups are scheduled via `dune-ctl backup schedule`. The entry lives
in the `dune` user's crontab and runs at 03:00, keeping the 14 most recent
bundles. Check with `dune-ctl backup schedule --show`.
