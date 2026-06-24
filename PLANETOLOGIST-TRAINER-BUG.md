# Planetologist Advanced Trainer — stuck questline (DB investigation)

**Status:** Diagnosis updated (2026-06-03). Confirmed known Funcom bug: the
Tier-2 Planetologist contract **"Adhering to Hierarchy"**
(`ct_trainer_planetologist2_02a`) can fail to spawn at Derek's camp after Buried
Archives when the Arrakeen social hub is not available. Two reversible tag
experiments ruled out the visibility and out-of-sequence-Kynes flags (both
reverted to baseline). The successful recovery was to prewarm `SH_Arrakeen`
through `ServerSetScale`/director availability, then re-open Derek's camp
conversation; the first conversation still showed no new branch, the second
showed the new **"Oh?"** dialogue and the mission advanced. DB editing
intentionally stopped; **escalate to Funcom**.
**Severity:** Character-blocking for the affected questline unless required social hubs are running
**Affects:** Dune: Awakening self-hosted server (Linux, app `4754530`, DB schema
`1973075-0-shipping`). Likely reproducible on official servers too — the relevant
state is server-side persisted data, not anything self-host-specific.

---

## Symptom (player report)

Planetologist Advanced Trainer questline (trainer NPC **Derek Chinara**) will not
advance after the **Imperial Testing Station 197** step (Western Vermillius Gap —
the Advanced Contract 1 "Minimic Film" retrieval that unlocks Tier 2 skills):

- Player completed Station 197 and received training.
- Player traveled to **Imperial Testing Station 76** (a later step in the chain,
  where Derek is normally imprisoned) — Derek not present there.
- Derek **never offered the Arrakeen step** (speak to Cyprian Io in the Salusan
  Bull bar re: Kynes' research).
- At Derek's South Hagga Basin camp, exhausting dialogue / walking away and
  returning / re-running prior stations did **not** advance the quest while
  `SH_Arrakeen` was stopped.

The documented "bad design, just go back and re-talk to Derek" community
workarounds did **not** work until `SH_Arrakeen` was running. After Arrakeen was
prewarmed, re-opening Derek's camp conversation a second time exposed the "Oh?"
branch and the quest advanced.

## What we ruled out

- **In-memory / world-process desync.** We deleted and recreated the
  `Survival_1` game-server pod (clean reload of world state from Postgres).
  Pod returned healthy (`Running`, `Ready`), player relogged — **no change**.
  This means the stuck state is **persisted in the database**, not transient
  server memory.

---

## Where the state lives

Game world DB is Postgres, in the battlegroup namespace.

| Thing | Value |
|---|---|
| Namespace | `funcom-seabass-sh-db3533a2d5a25fb-silakw` |
| Postgres pod | `sh-db3533a2d5a25fb-silakw-db-dbdepl-sts-0` |
| Service | `sh-db3533a2d5a25fb-silakw-db-dbdepl-svc` → `10.43.171.19:5432` |
| Database | `dune` (schema `dune`) |
| DB build/schema tag | `1973075-0-shipping` |

Read-only access pattern used (credentials come from the pod's own env):

```sh
ns=funcom-seabass-sh-db3533a2d5a25fb-silakw
pod=sh-db3533a2d5a25fb-silakw-db-dbdepl-sts-0
sudo kubectl exec -n $ns $pod -- sh -c \
  'psql -U "$POSTGRES_USER" -d dune -c "<query>"'
```

### ID spaces (important — they are NOT the same number)

- `account_id` — FK to `dune.encrypted_accounts(id)`. **The player's account is `2`.**
- `player_id` — FK to `dune.actors(id)`. **The player's actor is `4`.**

Quest/journey and tag tables key on `account_id`; tracked-card and dialogue
tables key on `player_id`. Mixing them up yields empty results.

---

## Relevant schema (reverse-engineered, read-only)

The world DB is a **clean relational schema**, not opaque blobs — quest/trainer
state is queryable and (in principle) editable. Key tables:

| Table | Keyed on | Holds |
|---|---|---|
| `journey_story_node` | `account_id`, `story_node_id` (text) | Main/side-quest "Journey" graph; per-node `complete_condition_state` / `reveal_condition_state` / `fail_condition_state` as JSONB, `has_pending_reward`. |
| `journey_tracked_cards` | `player_id` | Currently tracked journey + landsraad card. |
| `dialogue_taken_nodes` | `player_id`, `node_id` (int) | Every dialogue node the player has taken. |
| `dialogue_met_npcs` | `player_id`, `npc_name` | NPCs met. |
| `player_tags` | `account_id`, `tag` (text) | **Free-form progression/state flags — this is where trainer-quest progress lives.** |
| `tutorials` / `tutorial_per_player` | — | Tutorial completion. |
| `specialization_keystones_map` / `purchased_specialization_keystones` | — | Skill keystones unlocked. |
| `encrypted_player_state` | `account_id` | 1 row, ~192 kB **encrypted blob** — do NOT touch. |

**Key finding:** the Planetologist *trainer* questline is **not** stored in
`journey_story_node` (no journey node contains "planet" except Dunipedia codex
entries). It is tracked entirely via **`player_tags`**.

---

## Affected state — `player_tags` for `account_id = 2`

Query:

```sh
sudo kubectl exec -n $ns $pod -- sh -c \
  'psql -U "$POSTGRES_USER" -d dune -tAc "SELECT tag FROM dune.player_tags
   WHERE account_id=2 AND (tag ILIKE '\''%planetolog%'\'' OR tag ILIKE '\''%derek%'\''
   OR tag ILIKE '\''%chinara%'\'') ORDER BY tag;"'
```

Result (verbatim):

```
Contract.Target.Dialogue.DerekChinara.Contract4.KynesInvestigationCompleted
Contract.Target.Dialogue.Planetologist1.Contract1.Delivery
Contract.Target.Dialogue.Planetologist2.LearnedPhase2
Contract.Tracking.Completed.AdvancedPlanetologistTrainer.Contract1
Contract.Tracking.Completed.Planetologist1A
Contract.Tracking.Completed.Trainer_Planetologist1_01
DialogueFlags.Contracts.DerekChinara.Visibility.InvisibleAfterAdvContract1
DialogueFlags.Contracts.PlanetologistT1Complete
```

### Decoded

| Tag | Interpretation |
|---|---|
| `Contract.Tracking.Completed.Trainer_Planetologist1_01` | Basic trainer contract 1 complete |
| `Contract.Tracking.Completed.Planetologist1A` | (basic stage A) complete |
| `DialogueFlags.Contracts.PlanetologistT1Complete` | **Tier 1 complete** |
| `Contract.Target.Dialogue.Planetologist1.Contract1.Delivery` | Tier 1 delivery dialogue flag |
| `Contract.Target.Dialogue.Planetologist2.LearnedPhase2` | Tier 2 phase-2 learned |
| `Contract.Tracking.Completed.AdvancedPlanetologistTrainer.Contract1` | **Advanced Contract 1 (= Station 197 Minimic Film) complete** → unlocks Tier 2 |
| `Contract.Target.Dialogue.DerekChinara.Contract4.KynesInvestigationCompleted` | Kynes/Arrakeen investigation flag — **already set** (inconsistent: player never did the Arrakeen step) |
| `DialogueFlags.Contracts.DerekChinara.Visibility.InvisibleAfterAdvContract1` | **Sets Derek Chinara invisible / non-interactive after Advanced Contract 1** |

---

## Hypothesis

Two suspicious flags explain the stall:

1. **`DialogueFlags.Contracts.DerekChinara.Visibility.InvisibleAfterAdvContract1`**
   — After completing Station 197 (Advanced Contract 1), the game flips Derek to
   "invisible." The trigger that is supposed to make him **reappear at the next
   location to offer the next contract** apparently did not fire, so the player
   walks up to his camp and gets nothing interactable.

2. **`Contract.Target.Dialogue.DerekChinara.Contract4.KynesInvestigationCompleted`**
   is present even though the player never performed the Arrakeen / Cyprian Io
   "Kynes investigation" step. This is an **out-of-order / inconsistent flag**:
   the game may consider the Kynes investigation already done and is skipping the
   dialogue that would otherwise advance the chain — while Derek is simultaneously
   invisible, leaving no path forward.

Net: the trainer state machine is in a combination of flags it can't progress out
of — Derek is hidden, and the next milestone is marked complete without the
intervening contract being offered.

> We do **not** have Funcom's authoritative contract dependency graph, so the
> exact "correct" flag set for this point in the chain is inferred, not confirmed.

---

## Proposed remediation (NOT yet applied)

Reversible, backed-up experiment. **Not run yet — pending decision.**

1. Take a fresh full backup (in addition to nightly dump).
2. Record the exact tag value(s) before removal.
3. Stop `Survival_1` pod.
4. Most-likely-single fix: **remove** the visibility flag
   `DialogueFlags.Contracts.DerekChinara.Visibility.InvisibleAfterAdvContract1`
   so Derek becomes interactable again, then relog and check whether he offers
   the next contract.
5. If no improvement, consider the `KynesInvestigationCompleted` flag.
6. Restart pod; player relogs and tests.
7. On any regression: re-insert the removed tag, or restore from backup.

Risk: editing live game state without Funcom's contract graph could leave the
trainer line in a different inconsistent state. Tag edits are individually
reversible and the server has a single player, so blast radius is contained, but
the supported fix remains a Funcom support ticket.

---

## Remediation applied (2026-06-02)

Player was offline (`serverstats` = 0 players on Survival_1 and Overmap).

1. Fresh full backup taken:
   `/srv/backups/dune/live/sh-db3533a2d5a25fb-silakw/20260602-053129`
   (DB dump `pre-planetologist-tagfix-*.backup` + k8s metadata).
2. Pre-edit snapshot of all 264 `account_id=2` tags saved alongside it:
   `player_tags-account2-PRE-EDIT.txt`.
3. Removed exactly one tag in a transaction (before=1, DELETE 1, after=0):

   ```sql
   DELETE FROM dune.player_tags
   WHERE account_id=2
     AND tag='DialogueFlags.Contracts.DerekChinara.Visibility.InvisibleAfterAdvContract1';
   ```

4. All 7 other Planetologist/Derek tags left intact (verified post-delete).
5. No pod restart needed — offline-player tags load on player connect, so the
   value is re-read fresh on next login.

**Rollback** if this regresses or doesn't help:

```sql
INSERT INTO dune.player_tags(account_id, tag)
VALUES (2, 'DialogueFlags.Contracts.DerekChinara.Visibility.InvisibleAfterAdvContract1')
ON CONFLICT DO NOTHING;
```

or restore the `20260602-053129` bundle.

**Result (experiment 1 — visibility flag):** Derek **became visible again at
Imperial Testing Station 197** (confirms that tag controls his 197 appearance),
but he only says **"goodbye"** — no contract offered — and he is still **not**
present in his cage at Station 76. So the visibility flag was a *symptom*, not
the root cause. Quest did **not** advance. Visibility flag left removed (harmless;
reversible via the INSERT above).

---

## Comparative analysis (read-only) — the structural anomaly

Dumped all `Contract.*` / `DialogueFlags.*` tags for `account_id=2` and compared
trainer lines.

**Healthy completed trainer line (template), NPC ZaynDeWitte:**

```
Contract.Tracking.Completed.ZaynDeWitte.Contract1
Contract.Tracking.Completed.ZaynDeWitte.Contract2
Contract.Tracking.Completed.ZaynDeWitte.Contract3
Contract.Tracking.Completed.ZaynDeWitte.Contract4
+ Contract.Target.Dialogue.ZaynDeWitte.Contract2.ReturnToZayn / Contract3.ReturnToZayn /
  Contract4.CompletedAny / Contract4.ReturnToZayn
```

Sequential 1→2→3→4 completion tags, each with turn-in dialogue flags.

**Planetologist / Derek Chinara line (broken):**

```
Contract.Tracking.Completed.AdvancedPlanetologistTrainer.Contract1   (only Contract 1)
  -- NO Contract2 / Contract3 / Contract4 completion tags --
Contract.Target.Dialogue.DerekChinara.Contract4.KynesInvestigationCompleted  (a Contract 4 sub-flag!)
```

**Anomaly:** a **Contract 4** sub-objective dialogue flag
(`KynesInvestigationCompleted`) is set while only **Contract 1** is actually
complete — Contracts 2 and 3 never occurred. A flag was written **out of
sequence**. Derek's dialogue branch likely keys on the Kynes-investigation flag
being *absent* to offer the next (Arrakeen) contract; its erroneous presence
sends him down a path that assumes Contracts 2–3 are done, leaving him with no
valid line → "goodbye".

This is a strong candidate for the actual Funcom bug: **a downstream contract
sub-flag is set without the intervening contracts completing.**

## Proposed experiment 2 (pending decision)

Remove the single out-of-sequence flag and retest:

```sql
DELETE FROM dune.player_tags
WHERE account_id=2
  AND tag='Contract.Target.Dialogue.DerekChinara.Contract4.KynesInvestigationCompleted';
```

Rollback: re-INSERT the same `(2, tag)` row, or restore the `20260602-053129`
bundle. **Decision:** do not edit further flags beyond this one without Funcom
guidance — past this clear anomaly it becomes blind brute-forcing of the dialogue
state machine.

### Experiment 2 applied (2026-06-02, backup `20260602-055842`)

Player offline (logged out at Hagga South camp; `serverstats` = 0). Pre-edit
snapshot: `player_tags-account2-PRE-EDIT.txt` in that bundle (265 tags — the play
session between experiments added unrelated `BigMoments.Bike.Trigger` /
`BigMoments.Stillsuit.Trigger`; the game did **not** re-add the visibility flag).

One transaction:

```sql
INSERT INTO dune.player_tags(account_id, tag)            -- revert exp 1
VALUES (2, 'DialogueFlags.Contracts.DerekChinara.Visibility.InvisibleAfterAdvContract1')
ON CONFLICT DO NOTHING;                                  -- INSERT 0 1
DELETE FROM dune.player_tags                             -- exp 2
WHERE account_id=2
  AND tag='Contract.Target.Dialogue.DerekChinara.Contract4.KynesInvestigationCompleted';  -- DELETE 1
```

Confirms operator observation, with an important clarification on the two Derek
instances:

- **Station 197 "ghost" Derek** (the one our exp-1 visibility-flag removal exposed)
  — only says **"goodbye"**. This is the instance the visibility flag governs.
- **Hagga South camp Derek** — is **always present and interactable**, but offers
  only **basic / default dialogue** (appears to be the standard first-encounter
  lines), *not* the advanced-contract continuation. His camp dialogue state is
  effectively stuck at the intro tier.

In-game quest text still read as "turn in the film" despite the film being turned
in — consistent with the Contract-4 Kynes flag being set prematurely.

**Result (experiment 2):** **No change.** Camp Derek still offers only basic
dialogue; no new contract; Station 76 still empty. Removing the Kynes flag did
not unblock the chain.

---

## Root-cause conclusion (diagnosis complete; DB editing stopped)

In-game journal for **PLANETOLOGIST: ADVANCED — BURIED ARCHIVES** shows the
contract **fully complete**, all objectives checked:

```
[x] Recover the Minimic Film records from the testing station
[x] Deliver the Minimic Films to Derek
[x] Receive training from Derek at his camp
    "Additional Contracts will become available"
```

The promised "Additional Contracts" never spawn. World-data confirms why this is
not a location/marker problem:

- Marker-type census of `dune.markers`: exactly **one** `TrainerPlanetologist`
  marker exists, payload `WORLD_MAP_LOCATION_HaggaBasinSouth_ChinarasCamp`.
  There is **no second Derek location** and **no pre-placed next-contract
  marker**. (`SalusanBull` / Arrakeen markers exist but nothing links a pending
  planetologist contract to them.)
- `player_markers` (player_id=4) has **no** planetologist/contract marker
  awaiting discovery.

So the next advanced contract is meant to be **spawned dynamically by the
contract system from player flags/dialogue state** — and that spawn is not
firing.

**Most likely root cause:** a **missing turn-in flag**. Healthy trainer lines
(e.g. ZaynDeWitte) carry, for every contract, *both* a
`Contract.Tracking.Completed.<NPC>.ContractN` **and** a
`Contract.Target.Dialogue.<NPC>.ContractN.ReturnTo…/CompletedAny` turn-in flag.
Derek's advanced line has the completion tag
(`Contract.Tracking.Completed.AdvancedPlanetologistTrainer.Contract1`) **but no
corresponding "training received / turn-in" dialogue flag**. Instead the game
erroneously wrote `Contract.Target.Dialogue.DerekChinara.Contract4.KynesInvestigationCompleted`
(a *Contract 4* sub-flag) at the point training was received. Net: when the
player received training for Advanced Contract 1, the game set the **wrong flag**,
so the prerequisite that makes "Additional Contracts become available" was never
satisfied.

**Why we stopped here (deliberate):** the remaining plausible fix is to **add**
the correct turn-in flag the game failed to set — but its exact tag name is not
known from the data we have, and inserting a guessed tag name into a live
character is open-ended and risky (unlike removing a single clearly-anomalous
flag). Brute-forcing unknown tag names is out of scope without Funcom guidance.

### Current character state (post-experiments)

- Visibility flag `…InvisibleAfterAdvContract1`: **restored** (normal).
- `…Contract4.KynesInvestigationCompleted`: **restored** — experiment 2 (removal)
  had no effect, so it was put back to keep the character byte-identical to the
  original bug state for the Funcom report. Pre-edit snapshots preserved in
  `…/20260602-053129/` and `…/20260602-055842/`.
- Net effect of all experiments on Planetologist/Derek tags: **zero** (back to
  baseline). Only residue is two unrelated gameplay tags (`BigMoments.*`) the game
  itself wrote during the test logins.

### Recommended path

1. **File with Funcom** using the section below — this is a server-side
   contract-progression bug, not self-host-specific.
2. Hold further DB edits pending their guidance. If Funcom can supply the correct
   turn-in tag name (or confirms a safe re-trigger), the edit is trivial and
   fully backed up.

### Riskier option NOT taken (documented for completeness)

Soft re-trigger: delete `Contract.Tracking.Completed.AdvancedPlanetologistTrainer.Contract1`
to make Derek re-offer Buried Archives, hoping a clean re-completion sets the
correct turn-in flag. Rejected for now — could strip the Tier-2 unlock or produce
a worse inconsistent state; only attempt with a fresh backup and ideally Funcom
sign-off.

---

## Confirmed: known Funcom bug + likely trigger (2026-06-02 research)

The blocked contract is **"Adhering to Hierarchy"** (internal id
`ct_trainer_planetologist2_02a`), the **Tier-2** Planetologist trainer contract
that should follow Buried Archives.

**Intended chain** (per community guides):

1. **Buried Archives** — Testing Station 197 (done; unlocks Tier-2 skills).
2. **Adhering to Hierarchy** (`ct_trainer_planetologist2_02a`) — Derek offers it
   at his **Hagga Basin South camp**. NOTE: Chinara's Camp sits **directly above
   Imperial Testing Station No. 2**, so guide phrasing "offered near Station 2"
   and "at his camp" refer to the **same location**. This is exactly where the
   player already is — so the contract is failing to spawn *in place*, not a
   "go to a different location" problem.
3. **Science Unlocked** — Derek relocates to **Station 76**, locked in a cage.

**This is a documented, unfixed bug.** Steam thread
(`/app/1172710/discussions/0/501704624676410887/`): after this step Derek "only
says goodbye, no additional quests" despite "additional contracts will become
available," and fails to appear at later stations. No Funcom fix in-thread; at
least one player reports the **identical break on the Advanced Trooper** line →
systemic trainer-chain bug, not character-specific.

**Testing-station ("Ecolab") completion tags on this character** (for reference —
"Ecolab" == Imperial Testing Station internally):

```
Contract.Tracking.Completed.Ecolab002_Delivery_Vials      (Station 2)
Contract.Tracking.Completed.Ecolab029.CHOAMEquipment      (Station 29)
Contract.Tracking.Completed.Ecolab076_PlantBook           (Station 76)
Contract.Tracking.Completed.Ecolab197_KillNumber_Boss_01  (Station 197)
Contract.Tracking.Completed.Ecolab_010_kill_boss          (Station 10)
Contract.Tracking.Journey.EcolabCompleted
```

> NOTE: An earlier draft hypothesised these stations were completed "out of band"
> (independently of the trainer), desyncing the chain. **The player confirms this
> is NOT the case — the stations were done in normal progression.** That
> hypothesis is withdrawn. These tags are simply the completion records and do not
> indicate the trigger.

**Root trigger: unknown / Funcom-side.** The only Tier-2 trace on the character is
`Contract.Target.Dialogue.Planetologist2.LearnedPhase2`; no
`Trainer_Planetologist2_*` completion tags exist, and "Adhering to Hierarchy"
(`ct_trainer_planetologist2_02a`) left no trace anywhere in the DB. The spawn
condition lives in game-content logic we cannot inspect, so the precise reason it
fails to arm is undetermined from server data alone.

### Multi-Sietch topology — the strongest lead (2026-06-02)

**Correct terminology (per operator):**

- **Battlegroup = World** (e.g. official `Acheron`; ours is `Ixware`).
- **Sietch = a world instance / shard**, ~60-player cap each.
- Official worlds run **many** Sietches. Example — `Acheron` (Funcom,
  Washington D.C., build 1973075) lists **25 Sietches** (Abbir, al-Mut, Alraab,
  Barkan, Coanua, … Yaracuwan), ~1500-player aggregate cap.
- **`Ixware` (this self-hosted world) runs exactly ONE Sietch: `Silakwir`**
  (0/60).

(NB: the cave-altar `Sietch` *map markers* examined earlier are unrelated
exploration POIs — a red herring, retracted.)

The community workaround "visit other Sietches and return to Derek" means
**transferring to a different instance/shard**, which re-runs world/contract spawn
logic on arrival. **On a single-Sietch world there is no other instance to
transfer to**, so the workaround is structurally impossible here.

**This is the strongest explanation for the bug on this deployment:** the Tier-2
Planetologist contract spawn (or its recovery/re-evaluation path) appears to
assume a **multi-Sietch world** as on official servers. On a minimal single-Sietch
self-hosted battlegroup that assumption doesn't hold, so the contract never arms
and the usual instance-hop recovery is unavailable. This is Funcom-side logic that
is **specifically broken (or unrecoverable) on single-instance self-hosted
worlds.**

**Confirmed facts (operator + dune-status examples):**

- Self-hosted worlds **can and do run multiple Sietches.** Examples (build
  1973075): Galactica = 4 Sietches (cap 0/**240**), Holidyspice Test = 3
  (Amtal/Jacurutu/Thaddi, 2/**180**), Zekes Dune Haven / Last Sietch / Gp Net =
  2 each (0/**120**). **Capacity = Sietches × 60.** Ours: `Ixware` = 1 Sietch
  (`Silakwir`, 0/60).
- The **in-game client lets a player switch Sietches** under the world/battlegroup
  selector — confirmed on self-hosted. Character/progress is shared across the
  world's Sietches (you switch *instance*, same character).
- **Operational caveat:** before switching Sietches, **store vehicles (e.g. the
  ornithopter) in the backup/vehicle tool** — vehicles are instance-local and can
  be lost across a switch.
- Operator notes some self-hosters **"bought" additional Sietches online** →
  Sietch capacity may be governed by an **FLS-side entitlement**, not purely by
  local config.

**Live instance-scaling config (from the BattleGroup CR, per map set):**

```
EnableAutomaticInstanceScaling = true
MinServers      = 0      # floor of running instances
NumExtraServers = 0      # extra pre-warmed instances beyond demand  <-- likely the Sietch-count lever
grid            = 1x1    # single spatial partition per map
```

So Sietch instances scale on demand from a floor of 0 with no extra instances →
the single live Sietch. `NumExtraServers` (and any `MaxServers`) is the probable
local lever for a second Sietch.

**Research findings (2026-06-02):**

1. **NOT FLS-entitlement-locked — hardware-capped.** Funcom's official self-hosted
   page states a self-hosted World *can* run multiple Sietches; most hosts are
   "capped to 1 Sietch" only by **hardware**, not licensing. Multiple Sietches is
   a supported self-hosted configuration, and an official **Battlegroup Editor**
   UI exposes map/instance settings without hand-editing YAML. → The decisive
   unknown (entitlement gating) is resolved: **no entitlement gate.**

2. **Where Sietch count is controlled.** The director.ini (in the BattleGroup CR,
   `.spec.utilities.director.spec.configFiles.files."director.ini"`; template:
   `server/scripts/setup/update_maps.sh`) governs the **instanced content maps**
   only — `CB_*` (dungeons/ecolabs), `SH_Arrakeen`/`SH_HarkoVillage`,
   `DeepDesert_1`, `Story_*`, `DLC_*` — via `NumExtraServers` / `MinServers` /
   `EnableAutomaticInstanceScaling`. **The persistent Sietch world map
   `Survival_1` (and `Overmap`) have NO director.ini section** → they are not
   director-scaled. The Sietch count for the main world is therefore driven by the
   **BattleGroup set replicas**: live `sets[0]: map=Survival_1 replicas=1` → one
   Sietch (`Silakwir`).

3. **Refined lever for a second Sietch:** raise `Survival_1` set **replicas 1→2**
   (with the ServerSetScale chain, as `map-toggle.sh` does), NOT a director.ini
   edit. Confirm against Battlegroup Editor semantics if possible. (Official
   25-Sietch worlds presumably run a correspondingly higher `Survival` instance
   count.)

4. **Resource cost:** each extra `Survival_1` Sietch ≈ 5 Gi request / ~3.3 Gi RSS,
   plus the memory-focused-scheduler must bind the new pod. 64 GB host absorbs one
   more comfortably.

**Remaining to verify before/with the experiment:**

- Exact replica→Sietch mapping for `Survival_1` (set replicas vs. needing a
  `[ Survival_1 ]` director entry) — pin down via the Battlegroup Editor or a
  controlled test.
- Whether arriving on the new Sietch actually **re-runs the Tier-2 contract spawn
  gate** (the premise of the community workaround).

### Experiment result (2026-06-02) — replicas alone is INSUFFICIENT

Backup `20260602-073403`. Player offline. Patched `Survival_1` set 0
`replicas 1→2`.

What worked:
- Operator propagated correctly: ServerSet → `replicas=2, target=2`, a second pod
  `…-sg-survival-1-pod-1000000` was created and reached k8s `Running 1/1`.
  **pod-1 (the live world) was never disturbed.** No `ServerSetScale` involved
  (base map), so the BattleGroup set patch alone drove it.

What failed — the decisive finding:
- The second instance never reaches serving (`phase=Startup` forever), looping on:

  ```
  LogIgwDatabaseInterface: Error: LoadPartitionDefinition:
    Sql::load_world_partition(Survival_1, <world>, 0, 1000000) got 0 rows, expected exactly 1.
  LogIGW: Error: On partition loaded: FAIL!
  ```

- The replica uses its **index (1000000) as a world-partition id**, but the DB
  defines only partition **id=1** for Survival_1 (`worldPartitions: [{map:
  Survival_1, partitions:[{dimension:0, id:1, minX:0,maxX:1,minY:0,maxY:1}]}]`).
  `load_world_partition(..., dim=0, id=1000000)` → 0 rows → infinite fail loop.

**Conclusion:** **a Sietch ≠ a bare extra replica.** Each Sietch/instance needs a
matching **`world_partition` definition** the replica index can resolve to.
Bumping `replicas` without provisioning that partition just spawns a crash-looping
pod. This partition provisioning is what the official **Battlegroup Editor** must
do. Reverted to `replicas=1`; world healthy.

> Entitlement gating is ruled out (finding 1) and the operator does bring up extra
> instances (good), but **the missing piece is per-Sietch world-partition
> provisioning** — not yet cracked, and not something to brute-force on the live
> world map. Next research: how `worldPartitions` entries map to Sietches / how the
> operator derives a replica's partition id, ideally via the Battlegroup Editor.

### Confirmed Sietch model + why manual provisioning is non-trivial (2026-06-02)

A Sietch is defined by **two unique per-instance pieces**, both currently set as a
single value for our one Sietch:

1. **A unique name** — `Bgd.ServerDisplayName` in `UserEngine.ini`. Ours:
   `"Sietch Silakwir"` (one value). Operators confirm real worlds never show two
   Sietches with the same name → **names must be unique per Sietch**, and the stock
   `UserEngine.ini` comment says multi-Sietch naming must be done **"with the
   battlegroup editor."**
2. **A unique world partition** — `spec.database…worldPartitions` for the map.
   Ours: `Survival_1 → partitions:[{dimension:0, id:1, minX:0,maxX:1,minY:0,maxY:1}]}`
   (single `id=1`). A second instance needs its own partition entry it can resolve
   (the failed experiment looked for `id=1000000` and found none).

So provisioning a second Sietch = add a unique partition definition **and** a
unique `ServerDisplayName`, then scale — the bundle the **Battlegroup Editor**
performs. On commercial private hosts this is the seamless "name your Sietch, pick
the world to attach to" flow.

### CORRECTION: we already have the Battlegroup Editor — it is `bg-util`

An earlier draft wrongly said "we never got the Battlegroup Editor." **Wrong, and
worth flagging.** The Battlegroup Editor is **`bg-util`**, a Funcom Go TUI shipped
in our server package (`github.com/funcom/bg-util`, v1.0.16, at
`~/dune-server/server/scripts/bg-util`, symlinked `~/.dune/bin/bg-util`). It is
launched as the `KUBE_EDITOR` for `kubectl edit battlegroup`:

- Funcom Windows menu (`battlegroup.ps1`): `KUBE_EDITOR=…/bg-util kubectl edit
  battlegroup …`
- Our `server/scripts/battlegroup.sh` → `edit_battlegroup()` does the same;
  `edit_battlegroup_advanced()` opens the raw YAML in vi/nano.

`bg-util --help`: *"Edit dimensions and memory limits in a BattleGroup world
template. Opens a TUI to edit the main configuration spec."*

**Multi-Sietch mechanism, per bg-util's own model (decoded from the binary):**

- **A map's max servers (Sietches) = its `worldPartitions` ("dimensions") count.**
  Strings: *"Maximum number of servers this map can start (worldPartitions)"*.
- **`replicas` = servers actually started, and must be ≤ the partition count**
  (*"Actual number of servers that will be started (must be …)"*). This is exactly
  what our failed experiment violated: replicas=2 with only 1 partition → second
  instance had no partition (`id=1000000` not found) → crash loop.
- **Each partition/Sietch has its own name + password** (per-partition or shared):
  ops `perPartitionDisplayNameChanged` / `perPartitionPasswordChanged`; strings
  *"Set Bgd.ServerDisplayName"*, *"Bgd.ServerLoginPassword for %s"*,
  *"[All partitions] / shared across all partitions / edited per partition"*.
  Confirms Sietch names must be unique.

**Correct procedure to add a second Sietch (via the Editor):**

1. `server/scripts/battlegroup.sh` edit (launches `bg-util` TUI), or directly
   `sudo KUBE_EDITOR=~/.dune/bin/bg-util kubectl edit battlegroup <bg> -n <ns>`.
2. For `Survival_1`, **add a `worldPartitions` entry / raise max dimensions** →
   raises the server cap to 2.
3. Set **active servers (replicas) = 2**.
4. Give each partition a **unique `Bgd.ServerDisplayName`** (e.g. keep "Sietch
   Silakwir", add "Sietch <Name2>"); set passwords if desired.
5. Save/exit; operator reconciles and brings up Sietch #2 *with* a valid partition
   (no crash loop). ~5 Gi RAM headroom per Sietch.

This is the seamless multi-Sietch flow commercial hosts use — available to us all
along via `bg-util`.

### Assessment: dune-ctl does NOT cover the Battlegroup Editor (gap)

Operator concern is valid: **dune-ctl has no equivalent of `bg-util`.** dune-ctl
covers maps/settings/backup/public-ip/fls/health, but exposes **no** dimension/
partition (Sietch) management and **no** per-partition Sietch name/password
editing. That is why adding a Sietch meant hand-patching `replicas` (and failing)
instead of a supported operation. To reach parity:

- Short term: add `dune-ctl sietches edit` that **shells out to `bg-util`** (the
  official, safe TUI) — zero risk of re-implementing the partition math wrong.
- Longer term: native `dune-ctl sietches list|add|remove|scale|rename` that
  manipulates `worldPartitions` + `replicas` + per-partition `Bgd.ServerDisplayName`
  the way `bg-util` does, with the ≤-partition-count invariant enforced, plus
  capsule mirroring (so a cold-swap keeps the Sietch topology) and RAM preflight.
- Document the Editor (`bg-util`) and the worldPartitions=max-servers / replicas
  ≤ partitions / per-partition-name model in `dune-ctl/OPERATIONS.md`.

> Net: the multi-Sietch fix path is **NOT blocked** — `bg-util` does it safely
> today. The real gap is that dune-ctl never wrapped it, which is what made us
> improvise. Wrapping `bg-util` (or matching it) is the fix.

---

## Bottom line

- Player-facing symptom: Planetologist Tier-2 contract **"Adhering to Hierarchy"**
  (`ct_trainer_planetologist2_02a`) never spawns after **Buried Archives**;
  confirmed **known, unfixed Funcom bug** (reproduced by others; also seen on
  Advanced Trooper).
- All player prerequisites are satisfied; a game-server restart does not clear it;
  the spawn condition lives in game content, not the DB — no safe DB edit fixes it
  (two reversible tag experiments ruled out the obvious flags; character reverted
  to baseline).
- Most credible mechanism on *this* deployment: the recovery path relies on
  **switching Sietches**, impossible on our **single-Sietch** world. Adding a
  second Sietch is supported (hardware-, not entitlement-limited) and is done with
  the **Battlegroup Editor = `bg-util`** (which we have): add a `worldPartitions`
  entry (raises max servers), set replicas ≤ partition count, give each Sietch a
  unique `Bgd.ServerDisplayName`. A bare `replicas` bump (no partition) crash-loops
  — that was our error, not a missing tool.
- **dune-ctl gap:** dune-ctl has no equivalent of `bg-util` (no Sietch/dimension
  or per-partition name management) — the reason we improvised. Fix: wrap/match
  `bg-util`.
- **Actions outstanding:** (1) file the Funcom bug report; (2) add the second
  Sietch via `bg-util` and have the player switch Sietches to test the recovery;
  (3) optionally try the Arrakeen in-game workaround; (4) add `dune-ctl sietches`
  (wrap `bg-util` first, then native parity).

### Follow-up: add Sietch management to dune-ctl

**Design & plan: [`dune-ctl/SIETCHES-DESIGN.md`](dune-ctl/SIETCHES-DESIGN.md).**

Sietch (instance) count is currently only adjustable by hand-patching the
`Survival_1` set replicas + the ServerSetScale chain. Once the experiment below
validates the mechanism, **add first-class Sietch management to `dune-ctl`** so
this is a supported, safe operation rather than manual kubectl surgery. Sketch:

- `dune-ctl --world <w> sietches list` — show current Sietch instances, player
  counts, phase (distinct from `maps list`).
- `dune-ctl --world <w> sietches add|remove [--count N]` — add/remove Sietch
  instances. **NOTE (per experiment above): this must provision a matching
  `world_partition` definition for each new instance, not merely bump
  `Survival_1` replicas** — a bare replica bump crash-loops on
  `load_world_partition ... got 0 rows`. Implementation must replicate whatever
  the Battlegroup Editor does (add the `worldPartitions` entry the new replica
  index resolves to), then scale, with resource/headroom preflight (each Sietch
  ≈ 5 Gi req).
- `dune-ctl --world <w> sietches scale <N> --yes`.
- Mirror the value into the capsule source (like `maps persist`) so a cold-swap
  doesn't revert it.

Document the validated mechanism in `dune-ctl/OPERATIONS.md` and note the
single-Sietch → quest-bug interaction here. (Implement in `core/src/` — likely a
new `sietches.rs` alongside `maps.rs` — **once the world-partition provisioning
mechanism is understood**; the 2026-06-02 experiment proved replica-only scaling
is not enough.)

**In-game workaround confirmed here** (no DB edit):

- ~~Look for Derek near Station 2~~ — N/A: the camp **is** directly above Station
  2; the player is already at the correct location and the contract still does not
  spawn.
- Check **Station 197** for Derek (Tier-2 location), distinct from Station 76.
- Prewarm **Arrakeen** through the director/scaler path:
  `dune-ctl --world Ixware maps prewarm SH_Arrakeen --yes`.
- Re-open Derek's camp dialogue. In this case, the first conversation after
  Arrakeen came up still showed no new branch; the second re-initiation showed
  **"Oh?"** and advanced the mission.
- Keep both social hubs prewarmed for now:
  `dune-ctl --world Ixware maps prewarm SH_Arrakeen --yes` and
  `dune-ctl --world Ixware maps prewarm SH_HarkoVillage --yes`. A similar
  Swordmaster report mentions Harko/Arrakeen-style destination availability.

**Conclusion:** server-side data is consistent and editable, but the progression
bug is not fixed by direct DB tag surgery. The practical root cause is that
dialogue/quest gating can depend on social-hub destination availability before
the dialogue branch is evaluated. This is a Funcom-side trainer-chain bug because
the game should either allocate the required social hub before evaluating the
branch or evaluate the branch independently of whether the destination map is
already running.

---

## For a Funcom bug report

- Questline: **Planetologist Advanced Trainer** (NPC Derek Chinara).
- Repro point: completes **Imperial Testing Station 197 / Advanced Contract 1**
  (Tier 2 unlock), after which Derek never reappears to offer the next
  (Arrakeen / Kynes investigation) step.
- In-game journal shows "Buried Archives" **100% complete** (incl. "Receive
  training from Derek at his camp") with "Additional Contracts will become
  available" — but no further contract ever spawns.
- **Suspected root cause:** dialogue/quest gating checks the availability of the
  Arrakeen social hub before exposing Derek's next branch. With `SH_Arrakeen`
  stopped, Derek never exposed the Arrakeen/Cyprian step. With `SH_Arrakeen`
  prewarmed through the director/scaler path, re-opening Derek's dialogue exposed
  **"Oh?"** and the mission advanced.
- Secondary anomaly: compared to healthy trainer lines (e.g. ZaynDeWitte),
  Derek's advanced line still has the completion tag but lacks an obvious
  same-contract turn-in flag and has
  `Contract.Target.Dialogue.DerekChinara.Contract4.KynesInvestigationCompleted`.
  Removing that Kynes flag did **not** restore progression, so it is evidence of
  odd state but not the proven blocker.
- World data: only one `TrainerPlanetologist` marker exists (Chinara's Camp,
  Hagga Basin South); the next contract is spawned dynamically, not via a static
  marker — so it is not a "go to a different location" issue.
- Restarting the game-server process does not clear it (state is in Postgres).
- Removing the stray Kynes flag did **not** restore progression.
- Prewarming `SH_Arrakeen` did restore progression after the player re-opened
  Derek's camp dialogue a second time.
- DB build/schema tag: `1973075-0-shipping`.
