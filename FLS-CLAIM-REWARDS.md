# FLS Claim Rewards — What This Self-Host Can and Can't Do

Reference notes for the in-game **Escape menu → Claim Rewards** UI. Background
research only; this deployment uses progression-unlock for test-character setup
instead.

## The "Claim Rewards" UI is FLS-side, not server-side

The Claim Rewards list lives in the player's Funcom Live Services account, not
on this self-hosted battlegroup. Items shown there (PTC starter vehicles,
fuel, MK5 welding torch, etc.) were granted directly by Funcom against the
player's FLS account during PTC waves.

Evidence from this deployment:

- **FLS endpoints the director actually calls** (from
  `bgd-deploy` logs):
  - `api/Battlegroups_DeclareBattlegroupUpdates`
  - `api/Battlegroups_DeclareMaxPlayerCapacities`
  - `api/Battlegroups_DeclarePopulationAndActivity`
  - `api/Battlegroups_SendBattlegroupHeartbeat`

  All four are write-once registration/heartbeat endpoints. None grant items,
  manage entitlements, or push to player inboxes.

- **FLS JWT scope**: the `ServiceAuthToken` in each game pod's command line is a
  `ServiceHostType=2` (server) token. Per token-check, it authenticates the
  server *to* FLS for those four endpoints — it does not authorize granting
  entitlements *on behalf of* a player account.

- **No CRDs** in `server/images/operators/crds/` for entitlements, rewards,
  inboxes, or mail. The 27 Funcom CRDs cover BattleGroup, ServerGroup,
  ServerSet, ServerSetScale, Database*, MessageQueue, Filebrowser,
  FileOverlay, ImagePreheat, TextRouter — no reward/grant kind.

- **No references** in the repo to `entitle`, `reward`, `redeem`,
  `claim.*item`, `playerMail`, or `inbox` (only `landclaim` for base plots
  and JWT `claims`).

- **UserGame.ini / UserEngine.ini** expose PvP, security zones, mining
  multipliers, sandstorm, sandworm, building/landclaim limits — no starter
  inventory, no item grants, no reward sections.

So a private/self-hosted server admin **cannot** push items into the
Claim Rewards menu of an account.

## What community "welcome packages" actually do

"Welcome packages" reported on community/xworld servers must therefore reach
players through one of these in-server paths instead — none currently in use
here:

1. **Game admin/cheat console** — the `DuneSandboxServer` process is a UE5
   dedicated server. UE games typically expose a console with `additem`,
   `give`, `spawn`, `setlevel` style commands gated by an admin password or
   admin-list. We don't currently start the server with any
   `-AdminPassword=` / admin-list arg. Whether one exists for Dune is
   unconfirmed — would need to test by reading available console commands
   (e.g. `kubectl exec` into the game pod and inspecting `DuneSandboxServer`
   help output) or by trying common UE admin commands at the in-game console
   as an admin-logged player.

2. **Admin RMQ exchange injection** — the director already publishes to a
   `settingsUpdate` exchange bound to per-partition queues like
   `settingsUpdateQueue_Survival_11`. If there's a similar admin-grant
   exchange, an external tool authenticated to `mq-admin-svc:5672` could
   publish item-grant messages. Not observed in current traffic, would need
   passive inspection of the admin broker to discover.

3. **Direct Postgres insert** into player inventory tables under the `dune`
   schema. Fragile — depends on schema version, and the game server has
   in-memory inventory state that wouldn't see the row until reload. Not
   recommended.

4. **Authoritative starter loadout via game data** — Dune's starter inventory
   is defined in baked content (PAK files), not in INI overrides. Changing
   this would require patching shipped assets, not config.

## Why progression-unlock is the better path for this deployment

`UnlockProgression` (referenced in passing — a UserGame.ini or in-game admin
control that flips progression gates off) gives a test character access to
recipes/tech without inventory injection. That gets the same result for
testing (skip the grind to test mid/end-game features) without any of:

- FLS-side cooperation we don't have
- Game admin command discovery we haven't done
- Schema-coupled DB writes
- Asset patching

If we later want true "give X items to character Y" mechanics, the realistic
next step is item #1 — figure out what admin/cheat console Dune actually
exposes inside the game pod, and what password/admin-list arg activates it.
That investigation can stay paused until there's a concrete need.

## Quick-look commands if revisiting

```sh
# Verify no entitlement endpoints fire from director:
sudo kubectl logs -n funcom-seabass-sh-db3533a2d5a25fb-silakw \
  sh-db3533a2d5a25fb-silakw-bgd-deploy-... --tail=2000 \
  | grep -oE 'api/[A-Za-z_]+' | sort -u

# Inspect game pod save layout (no Mail/Reward dir; state is in Postgres):
sudo kubectl exec -n funcom-seabass-sh-db3533a2d5a25fb-silakw \
  sh-db3533a2d5a25fb-silakw-sg-survival-1-pod-1 -- \
  ls -la /home/dune/server/DuneSandbox/Saved/

# List Funcom CRDs (nothing reward/entitle/mail-shaped):
sudo kubectl get crd -o name | grep funcom.com
```
