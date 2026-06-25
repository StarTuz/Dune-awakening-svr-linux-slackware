# Off-Server Backup

Off-site replication of the Dune backup bundles. This closes the "1 offsite"
leg of the 3-2-1 rule that `BACKUP-RESTORE.md` left open: until now backups
lived only on `dune-vg/backups` (SSD LV), in the same machine as the live data.

> **Read `BACKUP-RESTORE.md` first.** This document only covers getting a copy
> *off the host*. The local bundle creation, restore procedure, and restore
> drills are documented there.

## Why, and what we are protecting against

The local backup volume is a different physical disk (`/dev/sdb2` SSD) than the
root/live data (`/dev/sdc2` HDD), so a single disk failure is already survivable.
Off-site exists to survive what a second disk in the same box cannot:

- Fire / theft / flood / power surge taking the whole machine.
- Host compromise or ransomware encrypting or deleting `/srv/backups`.
- An operator mistake (`rm`, LVM/btrfs error) wiping local backups.

Config and code are **already off-site** in the GitHub repo. Only the *game
data + secrets* (the backup bundles) lacked an off-site copy.

## Payload reality: this is a tiny-data problem

The whole `/srv/backups/dune` tree is ~22 MB. A DB dump is ~1.1 MiB; a full
bundle is a dump plus a few MB of metadata. Even years of retention stay in the
low hundreds of MB. **Storage and bandwidth cost are effectively zero at every
provider.** The design is therefore optimized for *robustness, immutability, and
low maintenance*, not capacity.

## Architecture (two independent legs)

```
/srv/backups/dune/live/<bg>/<timestamp>/   (dune:users, readable by cron)
        │
        ├── PRIMARY ──▶ restic repo on Backblaze B2  (rclone:b2-offsite:…/restic)
        │                 • client-side encrypted (AES-256) + dedup + `restic check`
        │                 • bucket Object Lock, 30-day Governance retention →
        │                   immutable; a compromised host cannot delete history
        │
        └── SECONDARY ▶ restic repo on Google Drive  (rclone:gdrive:dune-backups/restic)
                          • independent second restic repo, same passphrase
                          • client-side encrypted + dedup + `restic check`
                          • second vendor, uses existing Google One spend
```

Both legs are **restic repositories** reached through rclone's backends
(`b2-offsite`, `gdrive`). They share the one master passphrase, so there is a
single secret to escrow, and both are independently integrity-checkable.

Only the `live/` env subtree is replicated. PTC was the retired test
environment and is not backed up off-site. `system-snapshots/` and
`resource-snapshots/` are root-owned host-rebuild evidence (and the btrfs
snapshots are not portable files) — intentionally **not** sent off-site.

### Why both legs, and why restic for both

- restic/B2 is the *robust immutable* leg: integrity-verifiable and Object-Lock
  immutable, so it survives ransomware and is provably intact.
- restic/Google Drive is the *independent-vendor* leg: a second provider and a
  second restic repo, so a single-vendor outage, account lockout, or billing
  lapse does not lose the only off-site copy. It uses storage you already pay for.
- **Why two restic repos instead of restic + an `rclone copy` mirror:** the
  earlier design used `rclone copy` to a Drive `crypt` remote for the second
  leg, but rclone 1.74.3 hit an intermittent panic (in `fs/cache` during remote
  init) that made `rclone copy` unsafe for unattended cron. restic drives rclone
  through its stable `serve` path and never panicked, so both legs use restic.
  Bonus: the Drive copy is now deduplicated and integrity-checkable, not just a
  file mirror.

## ⚠️ The bundles contain secrets — encryption is mandatory

Each bundle's `k8s/*.yaml` includes the BattleGroup CR (which carries the **FLS
JWT** in its `arguments`), and `user-settings/` contains the sietch password,
admin password, and DB credentials. **Nothing leaves the host in plaintext.**
restic encrypts both repos client-side before upload (AES-256).

### Key escrow (the #1 way people lose backups)

A single master passphrase protects both repos. If the host is lost *and* the
passphrase is lost, the off-site copies are unrecoverable ciphertext. Therefore
the passphrase is escrowed in **two** places, neither of which is only the host:

1. Your password manager.
2. A printed copy in a physical safe / separate location.

On the host the passphrase lives only in `~/.dune/offsite-restic-pass`
(chmod 600), referenced by `RESTIC_PASSWORD_FILE`. The B2 and Drive access
credentials/tokens live (only) in `~/.dune/rclone.conf` (chmod 600).

## Files

| Thing | Path | Committed? |
|---|---|---|
| Sync driver | `scripts/offsite-sync.sh` | yes |
| This doc | `OFFSITE-BACKUP.md` | yes |
| Config (repo URLs, paths) | `~/.dune/offsite.env` (chmod 600) | **no** |
| restic passphrase | `~/.dune/offsite-restic-pass` (chmod 600) | **no** |
| rclone config (B2 key + Drive token) | `~/.dune/rclone.conf` (chmod 600) | **no** |
| Sync log | `~/dune-server/logs/offsite-sync.log` | no |

`scripts/offsite-sync.sh` reads all secrets from `~/.dune/offsite.env`; the repo
never contains credentials.

## One-time setup

### 1. Master passphrase (do this first, escrow it)

```sh
# Generate a strong passphrase, write it to the host file, then ALSO save the
# printed value to your password manager + a printed copy before continuing.
umask 077
restic generate --version >/dev/null 2>&1   # sanity: restic on PATH
openssl rand -base64 32 > ~/.dune/offsite-restic-pass
chmod 600 ~/.dune/offsite-restic-pass
cat ~/.dune/offsite-restic-pass     # <-- copy into password manager + print
```

### 2. Backblaze B2 (primary)

1. Create a Backblaze account, then a **private** bucket (globally-unique name,
   e.g. `dune-backups-offsite-<suffix>`). **Enable Object Lock at creation**
   (it cannot be added later).
2. On the bucket's Object Lock settings, set a **default retention of 30 days,
   Governance mode** (decided 2026-06-24). Every uploaded object is then
   immutable for 30 days, so a compromised host cannot delete recent history;
   Governance mode still allows a deliberate override with a privileged key.
   restic `prune` cannot remove locked objects — expected for this
   keep-everything leg.
3. Create an **application key** scoped to that one bucket with **Read and
   Write** access. (For defense-in-depth you can instead mint a key via the
   `b2` CLI that omits `deleteFiles`; Object Lock is the primary control and is
   sufficient on its own.) Note the `keyID` and `applicationKey` — shown once.

### 3. rclone remotes (B2 + Google Drive)

The B2 remote is non-interactive (just the keys); Drive needs browser OAuth.

```sh
# B2 (primary leg) — no browser needed:
RCLONE_CONFIG=~/.dune/rclone.conf rclone config create b2-offsite b2 \
    account "<keyID>" key "<applicationKey>"
chmod 600 ~/.dune/rclone.conf
```

The Google Drive remote needs an interactive browser OAuth, and `gdrive` is a
plain `drive` remote (no `crypt` wrapper — restic does the encryption). On a
headless host, forward rclone's callback port over SSH so the browser step on
your laptop authorizes the host's rclone:

```sh
# On your laptop:
ssh -L 53682:localhost:53682 dune@<host>
# Then on the host:
RCLONE_CONFIG=~/.dune/rclone.conf rclone config
#  n) new remote   name: gdrive   storage: drive
#     client_id / client_secret: blank
#     scope: 3 (drive.file)  — rclone only ever touches files it creates
#     Edit advanced config? n     Use auto config? y
#     -> open the printed http://127.0.0.1:53682/auth?... URL in your browser
#     Shared Drive? n             Keep remote? y
chmod 600 ~/.dune/rclone.conf
```

### 4. `~/.dune/offsite.env`

restic reaches both providers through rclone (`rclone:` backend); the B2 key and
Drive token live in `rclone.conf`, not here. Both repos use the one passphrase.

```sh
umask 077
cat > ~/.dune/offsite.env <<'EOF'
# Master passphrase encrypts BOTH restic repos (one secret to escrow).
export RESTIC_PASSWORD_FILE="$HOME/.dune/offsite-restic-pass"
# rclone config holds the b2-offsite and gdrive remotes.
export RCLONE_CONFIG="$HOME/.dune/rclone.conf"
# Two independent restic repos: Backblaze B2 (primary) + Google Drive (secondary).
export OFFSITE_REPOS="rclone:b2-offsite:<bucket>/restic rclone:gdrive:dune-backups/restic"
EOF
chmod 600 ~/.dune/offsite.env
```

### 5. Initialize and first push

```sh
scripts/offsite-sync.sh init        # init both restic repos
scripts/offsite-sync.sh run         # first backup to both repos
scripts/offsite-sync.sh snapshots   # confirm a snapshot in each
scripts/offsite-sync.sh check       # verify integrity of both
# limit any command to one repo with a substring filter:
scripts/offsite-sync.sh --repo b2 check
```

## Automation

The nightly local backup runs at 03:00 via the `dune` crontab
(`dune-ctl backup run --keep 14`). Chain the off-site push after it:

```cron
# existing local backup
0 3 * * *  DUNE_CTL_WORLD=sh-db3533a2d5a25fb-silakw /home/dune/dune-server/dune-ctl/target/release/dune-ctl backup run --keep 14  # dune-ctl-backup
# off-site replication, 20 min later (after the local bundle is written)
20 3 * * * /home/dune/dune-server/scripts/offsite-sync.sh run >> /home/dune/dune-server/logs/offsite-sync.log 2>&1  # dune-offsite-sync
```

A weekly integrity check is cheap insurance:

```cron
30 4 * * 0 /home/dune/dune-server/scripts/offsite-sync.sh check >> /home/dune/dune-server/logs/offsite-sync.log 2>&1  # dune-offsite-check
```

## Retention

Each repo keeps a snapshot per run; both dedup, so unchanged bundles cost almost
nothing. At MB scale, keep-everything is a defensible default.

- **B2 (primary):** the 30-day Governance Object Lock blocks deletion of recent
  packs, so `restic forget --prune` cannot reclaim them until the lock expires —
  intentional. To apply retention deliberately, use `offsite-sync.sh prune`
  (keep-daily 14, weekly 8, monthly 12, yearly 3) once objects are out of their
  lock window; Governance mode permits an override with a privileged key.
- **Drive (secondary):** no Object Lock, so `prune` reclaims normally.

```sh
scripts/offsite-sync.sh --repo gdrive prune   # safe anytime
scripts/offsite-sync.sh --repo b2 prune       # only past the 30-day lock window
```

## Restore from off-site (drill quarterly)

Off-site backups are not "proven" until a restore has been pulled from them.
restic restores byte-identical files; validate the dump itself with `pg_restore`
inside the postgres pod (the host has no `pg_restore`).

```sh
set -a; . ~/.dune/offsite.env; set +a
for repo in $OFFSITE_REPOS; do
  tgt=$(mktemp -d)
  restic -r "$repo" restore latest --target "$tgt"      # pull newest snapshot
  dump=$(find "$tgt" -name '*.backup' | head -1)
  file "$dump"                                           # expect: PostgreSQL custom database dump
  sha256sum "$dump"                                      # compare to the local original
  rm -rf "$tgt"
done
```

Then exercise an actual import against a disposable target per
`BACKUP-RESTORE.md` § Pre-Release Restore Drill. Record evidence and clean up
scratch dirs.

## Threat-model summary

| Scenario | Covered by |
|---|---|
| Single disk failure | local SSD/HDD split (pre-existing) |
| Host/site loss (fire, theft) | both off-site legs |
| Ransomware / host compromise deleting backups | restic/B2 bucket Object Lock (30-day Governance) — immutable |
| Single cloud vendor outage / account lockout | the *other* restic repo (B2 vs Google Drive) |
| Lost encryption key | passphrase escrow (password manager + printed) |
| Config / code loss | GitHub repo (pre-existing) |
| Unproven restore | quarterly off-site restore drill |

## Status (2026-06-24)

Live and verified. Both repos initialized and pushed; `restic check` clean on
both; restore drill confirmed byte-identical dumps from each. Automated via the
`dune` crontab:

```cron
20 3 * * * .../scripts/offsite-sync.sh run   >> .../logs/offsite-sync.log 2>&1  # dune-offsite-sync
30 4 * * 0 .../scripts/offsite-sync.sh check >> .../logs/offsite-sync.log 2>&1  # dune-offsite-check
```

Note: the very first B2 snapshot (`54ae5961`) captured both `live` and the now-
excluded `ptc` paths; it is tiny and Object-Lock-held, so it ages out — prune it
after the 30-day lock window.

## Open follow-ups

- Conan co-tenant has no off-site story yet (`/srv/backups/conan`, owned
  `conan:users`). The same pattern applies but must run as `conan`.
- Decide retention cadence, or accept keep-everything (dedup keeps it cheap).
- Wire the off-site push into `dune-ctl` (e.g. `dune-ctl backup run --offsite`)
  so the TUI Backups tab reflects off-site status.
