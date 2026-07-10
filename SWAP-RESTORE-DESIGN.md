# Swap-Restore Design ("B": seamless swap-back)

Status: **B0 + B1 built and exercised end-to-end (2026-07-10).** First real swap
into SlackSalusa with `--restore` ran the full path; the destructive import
succeeded (player-row count 0 → restored) and the world came up all-green. One
fix fell out of it (empty-db guard baseline — see below). Scoped 2026-07-08 after the first real
multi-world lifecycle (create → swap → transfer → swap-back → restore) was run
end-to-end by hand. This captures how to make **swapping back into a parked
world restore its data automatically**, so a swap-in is one step instead of the
manual stop→restore→start dance.

**Where it stands (2026-07-09):**
- **B0 (sudo-safe restore staging) — built.** `world-capsules.sh`
  `restore_database_for()` stages the dump through the filebrowser pod
  (`kubectl cp` into `/srv/DatabaseDumps`) and applies an import
  `DatabaseOperation` — only whitelisted `sudo -n kubectl`, no host-path sudo.
  Standalone entry: `world-capsules.sh restore --world-id <bg> --bundle <ts>`
  (dry-run default; refuses unless the battlegroup is stopped).
- **B1 (opt-in `swap --restore`) — built.** `world-capsules.sh swap --restore`
  (and `--restore-force`), plumbed through `dune-ctl worlds swap --restore` and
  `capsules swap --restore`. After activate: resolve latest bundle → empty-db
  guard (`db-credentials.sh data-check`) → stop + drain → `restore_database_for`
  → leave STOPPED with a `sietches start` reminder.
- **Validated non-destructively:** bundle/dump resolution, import
  `DatabaseOperation` passes server-side validation, `/srv/DatabaseDumps` is
  writable via kubectl, stopped-guard refuses a running world, the single-active
  invariant refuses a swap into the live world, and `data-check` reported 23888
  rows on live Ixware (so the empty-db guard correctly *refuses* a populated
  target). All under non-interactive sudo.
- **Done 2026-07-10:** real destructive import proven. `swap SlackSalusa
  --restore --apply` parked Ixware, activated SlackSalusa, and the empty-db guard
  correctly **refused** (it read the schema-init seed rows) — a *safe* stop
  before any write. Finishing via the standalone `restore --apply` ran B0's
  import for real (staged through filebrowser, import `DatabaseOperation`
  `Starting→Ongoing→Succeeded`); player rows went 0→restored (`data-check` 2995),
  `sietches start` + `preflight` = all-green.

**The one fix that fell out — empty-db guard baseline.** Schema-init seeds ~888
rows into a brand-new `dune` schema (`applied_patches`≈846 migrations, plus
`map_names`, faction/specialization lookups). So the original guard ("refuse if
*any* dune-schema rows") is never satisfied on a fresh world — plain `--restore`
would always trip it and demand `--restore-force`, defeating the point. Fixed:
`db-credentials.sh data-check` now counts only **player-domain** tables
(`encrypted_accounts`, `encrypted_player_state`, `building_instances`,
`buildings`, `inventories`, `player_respawn_locations`) via a `pg_stat_user_tables`
allowlist — 0 on a fresh seeded world, >0 once a character/base exists (SlackSalusa
read 2995 with one transferred character + two bases). A fresh swap-in now
auto-restores with plain `--restore`; only a genuinely populated target refuses.

Related: `MULTI-WORLD.md` (hot-swap model + validated manual swap-back),
`WORLD-CAPSULES.md` (capsule/activate), `BACKUP-RESTORE.md` (restore mechanics).

---

## Why this exists

The hot-swap model parks a world by **deleting its namespace + PVC** after a
final backup — so a parked world's data lives *only* in its backups.
`world-capsules.sh activate` now bootstraps a world's live state (provisions the
`dune` role+db, deploys UserSettings), but it brings the db up **empty**. So
today, swapping *into* a world you previously parked gives you an **empty world**
unless you manually restore its backup afterward.

`MULTI-WORLD.md` originally claimed a parked world was "restorable via a
swap-back" — true, but only with a manual restore step that was never wired in.
**B closes that gap:** on swap-in, if the target world has a prior backup and its
db is empty, restore its latest bundle automatically.

---

## Validated manual procedure (what B automates)

Proven on Ixware, 2026-07-08 (main character + both bases came back clean):

```sh
world-capsules.sh swap --to <bg> --apply     # parks active; activates <bg> EMPTY
dune-ctl --world <bg> sietches stop           # activate can leave spec.stop=false
dune-ctl --world <bg> backup restore <bundle> --yes   # requires battlegroup STOPPED
dune-ctl --world <bg> sietches start
dune-ctl --world <bg> preflight               # expect all-green
```

B collapses steps 2–4 into the swap when appropriate.

---

## Lessons learned (these shape the design)

1. **Activate brings up an EMPTY provisioned db.** Good — it's the clean target
   for a restore, and the "db is empty" check is the natural safety guard.
2. **Activate can leave `spec.stop=false`.** The world starts coming up; a
   restore must **stop it first** (restore refuses unless stopped) and drain game
   pods, then restore, then start.
3. **`dune-ctl backup restore` requires the battlegroup stopped** and is
   env-guarded (won't cross PTC↔Live). Keep both.
4. **Restored data lands in the `dune` schema** (not `public`) — verify presence
   with `dune`-schema table counts, not `public`.
5. **THE BLOCKER — restore staging needs un-whitelisted sudo.** Both
   `dune-ctl backup restore` and `battlegroup.sh import` stage the dump to the
   root-owned host path `/funcom/artifacts/database-dumps/<bg>/` using
   `sudo cp/mkdir/test/ls/stat`. This works in an interactive session (cached
   sudo timestamp) but **fails under the non-interactive NOPASSWD-only sudo**
   that automation (swap) runs with — the whitelist covers `kubectl`/`ctr`, not
   `cp`/`mkdir`. **B cannot be automated until this is reworked.**
6. **No zero-stakes sandbox for restore.** Unlike the DB/UserSettings bootstrap
   fixes (testable on an empty throwaway world), restore only does anything when
   there's real data. Test B against a world that has a **verified off-site
   backup** as a safety net (e.g. SlackSalusa), never a world's only copy.

---

## Design

### Phase B0 — make restore staging sudo-whitelist-safe (prerequisite)

Rework the dump-staging so a restore needs no sudo beyond the `kubectl`/`ctr`
whitelist. Current chain (to be replaced):

```
dune-ctl restore:  sudo cp <bundle dump> -> /funcom/artifacts/database-dumps/<bg>/<name>
battlegroup.sh import:  sudo test/ls/stat that host dir; copy -> <PVC>/Saved/DatabaseDumps; apply DatabaseOperation
```

**Recommended approach (mirror the backup / UserSettings-deploy patterns):**
stage the dump straight into the world's PVC via a pod that mounts it — the
**filebrowser pod mounts the PVC at `/srv`** (same pod we already `kubectl cp`
UserSettings into) — then apply the import `DatabaseOperation` directly via
`sudo -n kubectl`. No host-path sudo.

```
kubectl cp <bundle dump>  <ns>/<filebrowser-pod>:/srv/Saved/DatabaseDumps/<name>
kubectl apply -f <rendered import DatabaseOperation>   # via sudo -n kubectl
wait for DatabaseOperation phase=Succeeded
```

**Open questions to verify at implementation time:**
- Exact path the import `DatabaseOperation` reads from inside the PVC. The live
  import logged staging into `<PVC>/Saved/DatabaseDumps` — confirm that's the
  operation's input dir and the filename convention.
- The import `DatabaseOperation` spec/template (`server/scripts/setup/templates/
  databaseoperation.yaml`) — what fields select import vs dump, and the dump
  path. Render it directly rather than shelling to `battlegroup.sh import`.
- Whether a `.yaml` companion spec must accompany the dump (backups write one).
- Fallback if filebrowser is not the right mount: any pod mounting the PVC while
  the world is stopped, or a short-lived helper pod.

Deliverable: `dune-ctl backup restore` works end-to-end under non-interactive
sudo (kubectl-only). Prove it by restoring an existing world without an
interactive sudo session. This alone is a worthwhile fix even without B1.

### Phase B1 — wire auto-restore into swap (opt-in)

Add `--restore` to `world-capsules.sh swap` / `dune-ctl worlds swap` (and
`capsules swap`). When set, after activating the target:

1. Resolve the target's **latest** backup bundle (env-matched). If none → skip
   (brand-new world, e.g. first activate — leave the empty db, as today).
2. **Safety guard:** only proceed if the target `dune` schema is **empty**
   (table count 0). If it already has data, refuse — never clobber.
3. Ensure battlegroup stopped + game pods drained.
4. Restore the bundle (B0 path); keep the env-guard.
5. Start the world; print `preflight` reminder.

Keep it **opt-in** (`--restore`, default off) until proven. Dry-run prints which
bundle *would* be restored and the empty/non-empty guard result. Best-effort
logging parallel to the backup-retarget step.

**Empty-db guard details (as built + tuned 2026-07-10).** `db-credentials.sh
data-check` sums `pg_stat_user_tables.n_live_tup` for a **player-domain table
allowlist** in the `dune` schema (`encrypted_accounts`, `encrypted_player_state`,
`building_instances`, `buildings`, `inventories`, `player_respawn_locations`) —
0 on a fresh, schema-init'd, never-played world; >0 once a character/base exists.
It is deliberately **not** "all dune-schema rows": the first real exercise
confirmed schema-init seeds ~888 reference rows, which would have made that signal
always non-zero. The allowlist tolerates a renamed/absent table (it just
contributes nothing), so the guard fails safe on the remaining tables. Guard
refuses when the count is non-zero (or `unknown`); `--restore-force` overrides.

### Phase B2 — polish (built 2026-07-10, after B1 was proven)

- **Restore is now the default** for a swap. A bare `swap --apply` (and the TUI
  `S` action) restores the target's latest backup, because a parked world always
  comes up empty — the old opt-in default was a footgun (a plain swap silently
  gave you an empty world). `--no-restore` is the escape hatch for a deliberate
  fresh start; `--restore-force` still overrides the empty-db guard. Safe by
  construction: the guard skips when no backup exists and refuses a populated
  target. Plumbed through `world-capsules.sh swap`, `dune-ctl worlds swap`, and
  `dune-ctl capsules swap` (the opt-in `--restore` flag is gone from dune-ctl;
  the shell still accepts `--restore` as an explicit no-op).
- **TUI `S` swap** inherits the default (no flag passed), and its confirm-modal
  text now spells out: parks the online world, activates + restores the selected
  one, and leaves it STOPPED (start from the Sietches tab, then preflight).
- **Auto-start after restore: intentionally NOT done.** The swap leaves the
  restored world stopped as a verify point — confirm the restore (preflight)
  before it goes live. A bad restore should not auto-publish. Revisit only if the
  extra manual start proves annoying in practice.

---

## Safety model

- **Restore only into an empty db** — the single most important guard; makes an
  accidental double-restore or a swap-in of a live world a no-op refusal.
- **Only when a backup exists** for the target; otherwise skip (fresh world).
- **Env-guarded** (reuse `ensure_bundle_environment`); never cross PTC↔Live.
- **Default-on with a `--no-restore` escape** (since B1 was proven — B2); dry-run
  shows the exact bundle and guard result; source bundles are immutable
  (recoverable if anything goes wrong).
- **Never the world's only copy under test** — validate against a world with a
  verified off-site backup.

## Testing strategy

1. B0 standalone: restore an existing (or throwaway) world with **no interactive
   sudo** — confirm the kubectl-only staging path works.
2. B1: exercise `swap --restore` on the next swap **into SlackSalusa**, which
   carries a verified B2 + Google Drive backup as a safety net. Verify the
   `dune`-schema data returns and `preflight` is all-green. If it misbehaves,
   fall back to the manual restore.
3. Only after that, consider B2 / default-on.

## Scope summary

| Phase | What | Gates | Status |
|---|---|---|---|
| **B0** | Sudo-whitelist-safe restore staging (kubectl-cp into PVC + direct DatabaseOperation) | Prerequisite; independently useful | **built + proven** (real import Succeeded 2026-07-10) |
| **B1** | Opt-in `swap --restore`: activate → (empty-db guard) → stop → restore latest → leave stopped | Needs B0 | **built + exercised** (shell + dune-ctl); guard baseline fixed to player-domain tables |
| **B2** | Restore default-on (`--no-restore` escape) + TUI confirm text; auto-start deliberately deferred | Needs B1 proven on a real swap | **built** 2026-07-10 |

> B1 leaves the world **stopped** after restore (with a `sietches start`
> reminder) rather than auto-starting — a deliberate verify point for v1.
> Auto-start is a B2 consideration once B1 has ridden a real swap.
