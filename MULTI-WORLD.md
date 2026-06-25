# Multi-World (Hot-Swap) Design

Status: **Design / proposed.** No implementation yet. This captures the plan for
running more than one Live world/battlegroup on Arrakis so a separate character
can live on its own world.

Related docs: `WORLD-CAPSULES.md` (the cold-swap capsule model this extends),
`CLAUDE.md` (host/cluster facts), `BACKUP-RESTORE.md`, `OFFSITE-BACKUP.md`.

---

## Why this exists

Funcom limits accounts to **one character per world per account**. The main
character already occupies the Live world `Ixware`
(`sh-db3533a2d5a25fb-silakw`). A second character (e.g. one transferred in from
a G-Portal private world) therefore **cannot** live on Ixware — it must have its
**own** world/battlegroup. This is a hard account-model constraint, not a
preference.

Goal: run a second (and eventually Nth) Live world on this host, with exactly
one online at a time, swapped on demand via `dune-ctl`. One world = one of the
operator's characters.

---

## Decision: hot-swap, not simultaneous

Two options were assessed (2026-06-25). **Hot-swap wins decisively** for solo
self-hosting.

### Option A — both worlds running at once (rejected)

Technically possible (the control plane is multi-tenant), but a bad trade on
this box:

- **RAM does not close.** Measured live with one full world + the Conan
  co-tenant: **37 Gi used / ~17 Gi free of 54 Gi**. Per-map RSS has ballooned
  far past the old doc baseline:

  | Map | RSS (2026-06-25) | old doc baseline |
  |---|---|---|
  | `Survival_1` (Hagga Basin) | **10.9 Gi** | ~3.3 Gi |
  | `DeepDesert_1` | **9.7 Gi** | ~1.0 Gi |
  | `Overmap` | ~0.9 Gi | ~0.16 Gi |
  | `SH_Arrakeen` / `SH_HarkoVillage` | ~1.0 Gi each | — |

  A second world's *minimum* playable surface is Hagga Basin alone
  (~10.9 Gi) plus its own infra stack (~1.2 Gi) ≈ **+12 Gi → ~49 Gi of 54 Gi**,
  with no room left to ever start its Deep Desert / hubs. Two *full* worlds need
  ~24 Gi more than the host has → deep into swap, and a 10 Gi game server that
  is swapping is unplayable.

- **Host networking forces port juggling.** Game-server pods run
  `hostNetwork=true` — they bind host UDP ports directly. Dune's allotted range
  is only **7782–7790** (Conan owns 7777/7778/14001/27015). Two simultaneous
  worlds need disjoint port sets in that 9-port window.

- **Cluster-wide resource collisions.** The game RabbitMQ AMQP NodePort is
  pinned to **31982** (public, router-forwarded, firewall-opened). A second
  simultaneous world needs a different public AMQP port + a second router
  forward + a firewall change.

- **FLS concurrency is unverified** — whether one host account may have two
  battlegroups *online* at once with FLS is not proven.

- **Zero solo payoff.** The only thing "both online" buys is instant hopping
  between characters, which is meaningless when you are the only player and can
  only be in one world at a time anyway.

### Option B — hot-swap (chosen)

Only one world is online at a time; swap on demand.

- **Sidesteps every constraint above:** one world's RAM; reuse the *same* UDP
  ports, NodePort 31982, router forwards, and firewall; only one FLS
  registration online at a time (so the concurrency unknown never applies).
- **~80% already built** — this is an extension of the existing cold-swap
  capsule model (`~/.dune/capsules/<env>/<bg>/`, `world-capsules.sh
  park/activate`, `dune-ctl --world`, environment-stamped backups).
- **"Keep last-played active" is free** — the active capsule *is* whatever you
  last swapped to.
- **Cost:** a swap is stop-one + start-other = a few minutes of downtime plus
  the usual 5–10 min FLS re-declaration before the world is browser-visible.
  Fine for solo.

---

## What the control plane already supports (measured)

| Capability | State | Verdict |
|---|---|---|
| Funcom operators | 4 controllers, cluster-scoped, one namespace per battlegroup (multi-tenant by design) | ✅ handles N worlds |
| Custom scheduler | `memory-focused-scheduler.sh` binds pods across `-A` (all namespaces) | ✅ no change needed |
| Capsule model | cold-swap park/activate exists, env-keyed | ⚠️ needs multi-capsule-per-env |
| `dune-ctl --world` targeting | resolves `~/.dune/capsules/<env>/<bg>/` + legacy YAML | ✅ already multi-world aware |
| Backups | env+battlegroup-stamped, restore guard refuses env mismatch | ✅ per-world already |

The gap is **not** the cluster — it is that the capsule model assumes **one
capsule per environment** (`ptc`/`live`). Two Live worlds are both `env=live`.

---

## Architecture

```
~/.dune/capsules/live/
  sh-db3533a2d5a25fb-silakw/      # Ixware  (main character)   [ACTIVE today]
    capsule.env, battlegroup.yaml, fls-secret.yaml, rmq-secret.yaml, UserSettings/
  sh-db3533a2d5a25fb-<suffix2>/   # new world (transferred character)  [COLD]
    capsule.env, battlegroup.yaml, fls-secret.yaml, rmq-secret.yaml, UserSettings/
```

Invariant: **at most one Live capsule is "activated" (has a running namespace)
at any time.** Inactive capsules are parked as files + backups, not running
namespaces. The two worlds share the same package root, images, operators
(`v1.5.0`), public ports, and FLS host id — they differ only in battlegroup
name, namespace, FLS token/secret, world data (DB), and UserSettings.

---

## Build scope (proposed)

Script-first in `world-capsules.sh`, then wire into `dune-ctl`.

1. **Multi-capsule-per-env keying.** Capsule discovery/inventory already walks
   `~/.dune/capsules/<env>/<bg>/`; confirm nothing assumes a single `live`
   capsule. `activate`/`park` must operate per-battlegroup, not per-env.

2. **Swap command.** `world-capsules.sh swap --to <bg>` (and
   `dune-ctl worlds swap <world>` / TUI Worlds-tab action) that:
   - refuses if the target is already active;
   - backs up the currently-active world (env+bg stamped);
   - parks the active world (stop battlegroup, wait for game pods gone, delete
     namespace **only after backup + export exist** — same algorithm as
     `WORLD-CAPSULES.md` §Activation);
   - activates the target capsule (apply secrets + BattleGroup, wait ready);
   - prints the FLS-redeclaration reminder + `preflight`.

3. **Single-active guard.** Hard refusal to `activate` world B while world A's
   namespace still exists (prevents the NodePort/port/FLS collision that would
   occur if two Live worlds ran at once). Surfaced in CLI + TUI.

4. **Create flow for the 2nd world.** Already supported:
   `world-capsules.sh create --env live --name "<title>" --token "<new-token>"`
   renders capsule files only (nothing applied). Needs the new FLS token (see
   below). Six-letter suffix battlegroup name, never numeric.

5. **dune-ctl surface.** `worlds list` already shows capsules; add an active/
   cold marker and a `worlds swap <world>` verb; TUI Worlds tab (`1`) gains a
   swap action with confirmation. `token-check --world <bg>` already tracks each
   world's expiry independently.

6. **Per-world everything is already isolated** by namespace + capsule:
   backups (`/srv/backups/dune/live/<bg>/`), UserSettings
   (`~/.dune/worlds/<bg>/`), FLS/RMQ secrets, settings drift.

Generalizes to N worlds (N characters) — nothing above is two-specific.

---

## FLS token for the second world

Each battlegroup needs its **own** FLS token. To stand up world #2:

1. In the Funcom portal, generate a new self-host token. It is scoped to the
   same host id (`db3533a2d5a25fb`) but a **new battlegroup**, e.g.
   `sh-db3533a2d5a25fb-<suffix>`.
2. **Six-letter suffix, never numeric** — FLS rejects numeric suffixes with
   `Invalid Authorization to manage SelfHosted Battlegroup` (Ixware = `…-silakw`).
3. The token is a credential: it lives only in that capsule's `fls-secret.yaml`
   (600). `dune-ctl` rotation tooling is already per-world.
4. **Two expiries to track now** — run `dune-ctl token-check --world <new>`
   alongside Ixware's (expires 2027-06-22). Don't let the second lapse silently.

Same host + different battlegroup is fine for hot-swap because only one world is
ever online — the concurrent-registration question never arises.

---

## Character transfer runbook (G-Portal → new self-hosted world)

The transfer itself is an **official, in-client** operation (no DB surgery).
Sources: Funcom Help Center "Character Transfers" / "Self-Hosting FAQ", Funcom
"Server Migrations" news, hosting-provider guides. Verify against the live
client before spending a token — some of this is version-specific.

**Direction & rules**
- Private/Official → self-hosted is **allowed**; self-hosted → Official/Private
  is **not** (one-way, no path back).
- Transfer to self-host is a **move, not a copy**: the character is **deleted
  from the origin** (G-Portal) once moved.
- Costs **1 Transfer Token** (1 to start, +1 every 7 real days).
- **Moves:** character + inventory + bank contents.
- **Does NOT move:** bases and vehicles — back them up first with the in-game
  **Base Reconstruction Tool** / **Vehicle Backup Tool**, rebuild at destination.

**Procedure**
1. Build + create capsule for world #2 (new FLS token, six-letter suffix).
2. `world-capsules.sh swap --to <bg2>` (or `dune-ctl worlds swap`) to bring the
   new world online; wait for FLS re-declaration (~5–10 min) so it is
   **browser-visible** — the destination must appear in the in-client server
   list to be selectable.
3. Verify/enable the self-host "accept incoming transfers" setting. It lives in
   the `UserSettings` / Director layer (exact INI key TBD — confirm against the
   live `dune-ctl settings` catalog at build time).
4. On G-Portal: back up base + vehicles; stash valuables in inventory/bank.
5. In-client: be in **Hagga Basin** → **Servers** tab → press **Z** → select
   the new self-hosted world → confirm. The character + inventory move in.
6. Rebuild base/vehicles on Arrakis. Verify with `dune-ctl --world <bg2>`
   `preflight` / `status` / `players`.

**Caveats**
- Irreversible + token-costed → do **not** transfer on assumptions; confirm the
  move/deletion behavior in the client first.
- The destination must be online + browser-visible at transfer time (hot-swap
  it in beforehand).

---

## Open items before implementation

- Confirm the exact self-host **"allow incoming transfers" UserSettings key**
  against the live server (may already be in the `dune-ctl settings` catalog).
- Confirm `world-capsules.sh` inventory/activate has no hidden single-`live`
  assumption.
- Decide the **swap UX** surface: CLI verb name (`worlds swap`?), TUI binding,
  confirmation copy.
- RAM headroom is per-world fine (one world at a time), but note Deep Desert +
  hubs make a *single* world ~25 Gi now; keep an eye on total with Conan.
- Generalization to N worlds: capsule keying must not hardcode two.
