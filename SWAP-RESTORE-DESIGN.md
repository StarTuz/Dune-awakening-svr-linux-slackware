# Swap-Restore Design ("B": seamless swap-back)

Status: **Design / not yet built.** Scoped 2026-07-08 after the first real
multi-world lifecycle (create → swap → transfer → swap-back → restore) was run
end-to-end by hand. This captures how to make **swapping back into a parked
world restore its data automatically**, so a swap-in is one step instead of the
manual stop→restore→start dance.

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

### Phase B2 — polish (after B1 is proven)

- TUI Worlds-tab `S` swap: offer a "restore latest backup" checkbox/confirm line.
- Consider making `--restore` the **default** for a swap-in of a world that has
  backups (with `--no-restore` escape hatch), once B1 has ridden a few real
  swaps without surprises.

---

## Safety model

- **Restore only into an empty db** — the single most important guard; makes an
  accidental double-restore or a swap-in of a live world a no-op refusal.
- **Only when a backup exists** for the target; otherwise skip (fresh world).
- **Env-guarded** (reuse `ensure_bundle_environment`); never cross PTC↔Live.
- **Opt-in first**, dry-run shows the plan, source bundles are immutable
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

| Phase | What | Gates |
|---|---|---|
| **B0** | Sudo-whitelist-safe restore staging (kubectl-cp into PVC + direct DatabaseOperation) | Prerequisite; independently useful |
| **B1** | Opt-in `swap --restore`: activate → (empty-db guard) → stop → restore latest → start | Needs B0 |
| **B2** | TUI checkbox + consider default-on | Needs B1 proven on a real swap |
