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

Backups are separated by environment:

- `ptc`: Public Test Client worlds only.
- `live`: official live self-host worlds only.

PTC and Live character databases must never mix. `dune-ctl backup restore`
checks `MANIFEST.txt` and refuses to restore a bundle whose
`environment=<ptc|live>` marker does not match the current world environment,
even when the restore is invoked with a full path.

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
/srv/backups/dune/<environment>/<battlegroup>/<timestamp>/
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
  `/srv/backups/dune/ptc/sh-db3533a2d5a25fb-xyyxbx/20260515-203757`.
- `~/dune-server/scripts/dune-backup.sh` created a full bundle at
  `/srv/backups/dune/ptc/sh-db3533a2d5a25fb-xyyxbx/20260515-203837`.
- The database dump operation
  `sh-db3533a2d5a25fb-xyyxbx-dump-20260515-203837` reached `Succeeded`.
- The resulting dump was about 1.1 MiB at
  `/funcom/artifacts/database-dumps/sh-db3533a2d5a25fb-xyyxbx/sh-db3533a2d5a25fb-xyyxbx-20260515-203837.backup`.

## Restore

Restore **overwrites the live database**. Always stop the battlegroup first.

Current safety behavior:

- `dune-ctl backup restore --yes <bundle>` refuses to run while
  `BattleGroup.spec.stop` is not `true`.
- `dune-ctl` still delegates the destructive import to Funcom's
  `battlegroup.sh import`, but feeds the required confirmation itself after
  the explicit `--yes` CLI flag has been supplied.
- The restore path stages the selected `.backup` into both
  `/funcom/artifacts/database-dumps/<battlegroup>/` and the server PVC's
  `Saved/DatabaseDumps/` directory before creating a `DatabaseOperation` with
  `spec.action: import`.

### Preferred: dune-ctl backup restore

```sh
# 1. Check what's available
dune-ctl backup list

# 2. Stop the battlegroup
dune-ctl battlegroup stop

# 3. Restore (--yes required to prevent accidental invocation)
dune-ctl backup restore --yes <timestamp>
# or by full path:
dune-ctl backup restore --yes /srv/backups/dune/<environment>/<battlegroup>/<timestamp>

# 4. Start and verify
dune-ctl battlegroup start
dune-ctl preflight        # check the "gateway IP" row
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
   sudo cp /srv/backups/dune/<environment>/<battlegroup>/<timestamp>/database/<backup>.backup \
     /funcom/artifacts/database-dumps/<battlegroup>/
   ```

3. Run the import:

   ```sh
   ~/dune-server/server/scripts/battlegroup.sh import <backup>.backup
   ```

4. Start and verify:

   ```sh
   ~/dune-server/server/scripts/battlegroup.sh start
   dune-ctl preflight        # check the "gateway IP" row
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

## Restore Readiness Assessment

Assessed on 2026-05-19:

- Logical dump creation is working. Recent `DatabaseOperation` dump resources
  are `Succeeded`, and the latest bundle
  `/srv/backups/dune/ptc/sh-db3533a2d5a25fb-xyyxbx/20260519-003542` contains a
  PostgreSQL custom-format dump readable by `pg_restore --list`.
- Bundle metadata is complete enough for reconstruction context: BattleGroup
  YAML, DatabaseDeployment YAML, PV/PVC YAML, `kubectl get all`, deployed
  `UserSettings`, local `User*.ini`, diagnostics, and repo commit.
- Nightly backup scheduling is installed for the `dune` user:
  `0 3 * * *`, keeping 14 bundles.
- The live import prerequisites exist: `DatabaseOperation` CRD, server PVC with
  `role=igw-server`, and a resolvable local-path PV host directory.
- Physical Funcom backup resources (`DatabaseBackup`,
  `DatabaseBackupSchedule`, `DatabaseMigrate`, `DatabaseRestore`) are not in
  use. The proven layer is logical dump/import only.
- Backups remain local-only on `/srv/backups/dune`; off-host replication is not
  solved.
- Existing PTC bundles have been migrated to
  `/srv/backups/dune/ptc/sh-db3533a2d5a25fb-xyyxbx/` and stamped with
  `environment=ptc` in their manifests.

Scratch restore result:

- Restored
  `/srv/backups/dune/ptc/sh-db3533a2d5a25fb-xyyxbx/20260519-003542/database/sh-db3533a2d5a25fb-xyyxbx-20260519-003542.backup`
  into an isolated temporary Postgres database named
  `restore_drill_20260519` inside the existing Postgres pod.
- `pg_restore` exited successfully.
- Restored schema sanity counts: 161 `dune` tables, 520 `dune` functions,
  28 `world_partition` rows, 13 `farm_state` rows, 1 account, 1 player row,
  41 actors, 112 items, 0 guilds.
- The restored player row decrypted successfully as `Marinka`, status
  `Offline`, last login `2026-05-17 22:37:18.943276+00`.
- The temporary restore database and staged `/tmp/restore-drill-20260519.backup`
  were removed after validation.
- Live battlegroup remained Healthy with `Survival_1` and `Overmap` running.

Live import rehearsal result:

- Rehearsal performed before the expected PTC shutdown window on May 19, 2026.
- Created fresh backup bundle
  `/srv/backups/dune/ptc/sh-db3533a2d5a25fb-xyyxbx/20260519-041216`.
- The normal `dune-ctl backup run` path initially failed because
  `server/scripts/battlegroup.sh backup` uses `sudo mkdir/cp/sed` under
  `/home/dune/.dune`, and those commands are not passwordless for the `dune`
  user. `scripts/dune-backup.sh` now creates and waits for the
  `DatabaseOperation` directly with `sudo kubectl`, which matches the working
  manual path and avoids those sudoers gaps.
- The repaired `dune-ctl backup run --keep 14` path was then verified with
  bundle `/srv/backups/dune/ptc/sh-db3533a2d5a25fb-xyyxbx/20260519-043837`.
- The fresh dump operation
  `sh-db3533a2d5a25fb-xyyxbx-dump-20260519-041216` reached `Succeeded`, and
  `pg_restore --list` succeeded against the resulting dump.
- Stopped the PTC battlegroup. It reached `spec.stop=true`, phase `Stopped`,
  with zero running game server pods.
- Staged the fresh backup into the server PVC through the filebrowser pod at
  `/srv/DatabaseDumps/sh-db3533a2d5a25fb-xyyxbx-20260519-041216.backup`.
- Created Funcom import operation
  `sh-db3533a2d5a25fb-xyyxbx-import-20260519-042000`; it reached `Succeeded`.
- Restarted the battlegroup and reapplied `--RMQGameHttpPort=30196` through
  `dune-ctl gateway-patch` (historical — that gateway patch step was retired
  2026-06-02; verify the gateway IP via `dune-ctl preflight` instead).
- Final state: battlegroup `Healthy`, `Survival_1` and `Overmap` running,
  diagnostics OK, no players online.
- Live DB sanity after import: 28 `world_partition` rows, 2 `farm_state` rows,
  1 account, 1 player row, 41 actors, 115 items. The player row decrypted as
  `Marinka`, status `Offline`, last login `2026-05-19 02:44:31.082799+00`.
- Removed the staged restore file from the server PVC after the successful
  import.

Residual risks:

- A successful import has now been completed against the live PTC world, but not
  against a separate disposable full BattleGroup or the future official build.
- Official Live should start from a fresh database. PTC backups are retained for
  PTC forensics only and are not valid restore sources for Live.
- Logical restore validates database state, not the full host rebuild story.
  The full rebuild story also depends on repo state, server package, k3s/local
  path data, host firewall, and current Funcom scripts.
- `dune-ctl backup schedule --show` can report "not installed" inside restricted
  sandbox contexts where `crontab -l` cannot read the spool. Verify on the host
  directly when scheduling is in doubt.

## Pre-Release Restore Drill

Preferred drill, no live-world data loss:

Do not create a second full BattleGroup from the stock Funcom
`world-template.yaml` on this host without editing the template first. The
template pins the game RabbitMQ NodePort to `31982`, which collides with the
currently-active Live battlegroup (`Ixware` / `sh-db3533a2d5a25fb-silakw`) and
with the cold PTC capsule's reserved NodePort. Use an isolated Postgres
scratch restore for dump validation, or create the full disposable world on a
separate k3s host/VM or with audited port changes.

1. Create a fresh full backup:

   ```sh
   dune-ctl backup run --keep 14
   dune-ctl backup list
   pg_restore --list /srv/backups/dune/<env>/<bg>/<timestamp>/database/*.backup >/tmp/dune-restore-toc.txt
   ```

2. Build a disposable restore target before official launch. Best options, in
   order:

   - Fresh temporary BattleGroup/world with a separate namespace, ports, and FLS
     token.
   - Fresh k3s host or VM using the same server package and repo.
   - Live PTC world only during an explicit outage window, after taking a new
     backup and accepting rollback risk.

3. Stop the target battlegroup:

   ```sh
   dune-ctl --world <target> battlegroup stop
   ```

4. Restore the selected bundle:

   ```sh
   dune-ctl --world <target> backup restore --yes <timestamp-or-bundle-path>
   ```

5. Start and repair runtime-only state:

   ```sh
   dune-ctl --world <target> battlegroup start
   dune-ctl --world <target> preflight        # check the "gateway IP" row
   dune-ctl --world <target> status
   dune-ctl --world <target> settings status
   dune-ctl --world <target> players
   ```

6. Validate in game:

   - Server browser visibility after the normal FLS declaration delay.
   - Character presence and expected last known state.
   - Hagga Basin load.
   - Overmap travel.
   - Deep Desert travel after `dune-ctl --world <target> maps start DeepDesert_1`.

7. Record drill evidence:

   ```sh
   sudo ~/dune-server/scripts/resource-snapshot.sh restore-drill-YYYYMMDD
   dune-ctl --world <target> diagnostics
   dune-ctl --world <target> backup list
   ```

## Scheduling (resolved)

Nightly backups are scheduled via `dune-ctl backup schedule`. The entry lives
in the `dune` user's crontab and runs at 03:00, keeping the 14 most recent
bundles. Check with `dune-ctl backup schedule --show`.
