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

Use the host wrapper:

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

Restore is intentionally more manual than backup.

The existing destructive import path is:

```sh
~/dune-server/server/scripts/battlegroup.sh import <backup-name>
```

That command:

1. Confirms interactively.
2. Finds the server PVC.
3. Stages the selected dump into `Saved/DatabaseDumps`.
4. Creates a `DatabaseOperation` with `spec.action: import`.
5. Waits for the operator to report success or failure.

Recommended restore procedure:

1. Confirm the backup directory and inspect `MANIFEST.txt`.
2. Stop the battlegroup:

   ```sh
   ~/dune-server/server/scripts/battlegroup.sh stop
   ```

3. Copy the chosen backup file back into Funcom's staging path if needed:

   ```sh
   sudo mkdir -p /funcom/artifacts/database-dumps/<battlegroup>
   sudo cp /srv/backups/dune/<battlegroup>/<timestamp>/database/<backup>.backup \
     /funcom/artifacts/database-dumps/<battlegroup>/
   ```

4. Run the import:

   ```sh
   ~/dune-server/server/scripts/battlegroup.sh import <backup>.backup
   ```

5. Start the battlegroup:

   ```sh
   ~/dune-server/server/scripts/battlegroup.sh start
   ~/dune-server/scripts/gateway-patch.sh
   ```

   If you are resuming a completed update rather than doing a manual restore,
   `~/dune-server/scripts/update.sh --post-update-only --start-after` now wraps
   the same DB check, battlegroup start, and gateway patch sequence.

6. Verify with:

   ```sh
   dune-ctl status
   dune-ctl diagnostics
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
- Off-host replication target: NAS via `rsync`, cloud via `rclone`, or both.
- Restore drill cadence. A backup should not be considered proven until an
  import has been tested on a disposable battlegroup or fresh cluster.
