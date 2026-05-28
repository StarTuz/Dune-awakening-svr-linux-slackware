# World Capsules

Goal: keep PTC, Live, and any future Dune self-hosted world completely
separate, while allowing one world to be activated on demand on this host.

This is stricter than normal Kubernetes namespace separation. PTC and Live
character databases must never mix.

## Current Findings

Assessed on 2026-05-26 (`kubectl get battlegroups -A`, `dune-ctl worlds list`):

- **Current active world**: `Ixware` (`sh-db3533a2d5a25fb-silakw`),
  environment `live`, sietch `Sietch Silakwir`, region `North America`.
  The sietch password is intentionally left operator-managed and not set by
  `world-capsules.sh`.
- **Cold (inactive) capsule**: `Slackware-Arrakis`
  (`sh-db3533a2d5a25fb-xyyxbx`), environment `ptc`. Stored under
  `~/.dune/capsules/ptc/sh-db3533a2d5a25fb-xyyxbx/` and its legacy
  `~/.dune/sh-db3533a2d5a25fb-xyyxbx.yaml` spec.
- Current installed package roots:
  - `/home/dune/dune-packages/live/app-4754530/server`: Steam App `4754530`,
    build `23301681`, `Dune: Awakening Self-Hosted Server` (active).
  - `/home/dune/dune-server/server`: Steam App `3104830`, build `23243500`,
    `Dune: Awakening Public Test Client Server` (cold).
  - `/home/dune/steamcmd/dune_server`: Steam App `3104830`, build `23216207`,
    older PTC package copy.
- Live package validation (at activation):
  - Steam build: `23301681`
  - Battlegroup image tag: `1963158-0-shipping`
  - Operator image tag: `v1.5.0`
- Live package images have been imported into k3s/containerd for
  `1963158-0-shipping`. Funcom operators are `v1.5.0`.
- Current cluster has one battlegroup namespace:
  `funcom-seabass-sh-db3533a2d5a25fb-silakw`.
- Current public game RabbitMQ NodePorts (Live):
  - AMQP: `31982`
  - management HTTP: `30196`
- The stock `world-template.yaml` pins the game RabbitMQ AMQP NodePort to
  `31982`. Both PTC and Live capsules share these public NodePorts, which is
  exactly why they cannot run simultaneously.
- Live backups are written under
  `/srv/backups/dune/live/sh-db3533a2d5a25fb-silakw/` with manifests stamped
  `environment=live`. PTC bundles remain under
  `/srv/backups/dune/ptc/sh-db3533a2d5a25fb-xyyxbx/`.

Run current inventory:

```sh
scripts/world-capsules.sh inventory
```

## Isolation Boundaries

What namespaces isolate well:

- BattleGroup CRs.
- FLS and RMQ secrets.
- Postgres StatefulSet and database PVC.
- Server PVC with `Saved/`.
- Filebrowser, gateway, director, text-router, and message queues.

What namespaces do not isolate:

- CRDs are cluster-scoped.
- Funcom operators are cluster-scoped and run in `funcom-operators`.
- Container images are global in k3s/containerd.
- NodePorts are cluster-wide.
- Host firewall/router forwarding is host-wide.
- `/funcom/artifacts/database-dumps/<battlegroup>` is a shared host path with
  battlegroup subdirectories.

Practical consequence: separate namespaces are enough for data separation, but
not enough for PTC and Live package/operator version separation. If PTC and Live
ship incompatible CRDs or operators, they cannot safely coexist in one k3s
cluster.

## Safe Operating Model

Use cold-swappable world capsules first.

A capsule is:

- `environment`: `ptc` or `live`.
- Steam app ID and package root.
- Package build and image/operator versions.
- Battlegroup name.
- Namespace.
- World title.
- FLS token identity and secret YAML.
- RMQ secret YAML.
- Rendered BattleGroup YAML. This embeds the director config
  (`spec.utilities.director.spec.configFiles.files."director.ini"`), including
  per-map `MinServers` persistence. `dune-ctl maps persist` writes both the live
  CR and this capsule copy so a cold-swap re-activation keeps persistence
  settings; editing only the live CR would be reverted on activation.
- Per-world UserSettings profile.
- Backup root.
- Optional exported namespace evidence.

Only one capsule should be active until proven otherwise. Inactive capsules are
parked as files and backups, not as running Kubernetes namespaces.

## Directory Plan

```text
/home/dune/dune-packages/
  ptc/
    app-3104830/
      server/
  live/
    app-4754530/
      server/

/home/dune/.dune/capsules/
  ptc/<battlegroup>/
    capsule.env
    battlegroup.yaml
    fls-secret.yaml
    rmq-secret.yaml
    UserSettings/
    package-root -> /home/dune/dune-packages/ptc/app-3104830/server
  live/<battlegroup>/
    capsule.env
    battlegroup.yaml
    fls-secret.yaml
    rmq-secret.yaml
    UserSettings/
    package-root -> /home/dune/dune-packages/live/app-4754530/server

/srv/backups/dune/
  ptc/<battlegroup>/<timestamp>/
  live/<battlegroup>/<timestamp>/
```

The existing `~/.dune/<battlegroup>.yaml` discovery can remain as the active
world view for `dune-ctl`, but source-of-truth should move to
`~/.dune/capsules/<env>/<battlegroup>/`.

## Commands To Add

Script-first, then wire into `dune-ctl`:

```sh
scripts/world-capsules.sh inventory
scripts/world-capsules.sh create --env live --name <title> --token <token> --package-root <root>
scripts/world-capsules.sh package install --env live
scripts/world-capsules.sh package validate --env live
scripts/world-capsules.sh images load --env live
scripts/world-capsules.sh park --world <bg>
scripts/world-capsules.sh activate --world <bg>
scripts/world-capsules.sh delete-k8s --world <bg> --yes
scripts/world-capsules.sh export --world <bg>
```

Initial implementation should keep `create`, `park`, and `activate`
non-destructive unless passed `--apply`.

Implemented now:

- `inventory`
- `package install`
- `package validate`
- `images load`
- `create`
- `activate` dry-run, with `--apply` guarded against existing battlegroups

Live FLS world names must use the six-letter suffix form, for example
`sh-<hostid>-silakw`. Numeric suffixes are rejected by FLS with
`Invalid Authorization to manage SelfHosted Battlegroup`.

`create` renders capsule files only. It does not apply Kubernetes resources.

Live capsule creation prompts for, or accepts flags for:

- self-host token
- world title
- sietch name, default `Sietch Abbir`
- region, default `North America` for Live and `North America Test` for PTC
- package root, default `/home/dune/dune-packages/live/app-4754530/server`

Example:

```sh
scripts/world-capsules.sh create --env live
```

Or non-interactive:

```sh
scripts/world-capsules.sh create \
  --env live \
  --name "Official Arrakis" \
  --sietch-name "Sietch Abbir" \
  --region "North America" \
  --token "$FLS_TOKEN"
```

Smoke harness:

```sh
scripts/test-world-capsules.sh
```

Dry-run the prepared Live capsule activation:

```sh
scripts/world-capsules.sh activate --env live --world-id sh-db3533a2d5a25fb-silakw
```

## Activation Algorithm

For a cold swap:

1. Run `scripts/world-capsules.sh inventory`.
2. Confirm target capsule environment and package root.
3. Stop the active battlegroup.
4. Run a final backup into the active environment bucket.
5. Export current namespace manifests and PVC/PV evidence.
6. Delete or park the active battlegroup namespace only after the backup and
   export exist.
7. Point `~/.dune/download` at the target package root.
8. Ensure target package images are loaded into containerd.
9. Ensure operator/CRD version compatibility with the target package.
10. Apply target secrets and BattleGroup YAML.
11. Wait for namespace, DB, message queues, gateway, director, and game pods.
12. Apply gateway patch.
13. Verify `dune-ctl --world <target> status`, diagnostics, token, and login.

Do not restore PTC DB data into Live. Official Live starts fresh unless Funcom
provides a separate official character transfer path.

## Side-By-Side Requirements

Side-by-side PTC/Live is not the default plan. It requires all of these:

- Same compatible CRD/operator set.
- Unique public RabbitMQ AMQP NodePort per world.
- Unique public RabbitMQ HTTP management port per world if needed by gateway
  declaration.
- Host firewall/router forwards for the selected active public ports.
- Enough RAM for both worlds.
- Distinct FLS tokens, names, and data-center identity where Funcom requires
  that.

Until those are proven, assume side-by-side is unsafe and use cold swap.

## Dune-Ctl Wiring

After script behavior is proven:

- `dune-ctl` now reads `~/.dune/capsules/<env>/<bg>/capsule.env` alongside
  legacy world YAML specs.
- `dune-ctl capsules` is the supported wrapper for inventory, package
  validation, image loading, capsule creation, and activation.
- Add package/app/build fields to the status header.
- Keep backup restore environment enforcement in `backup.rs`.

## Current Guardrails

- `dune-ctl backup restore` refuses environment mismatches.
- PTC backups are under `/srv/backups/dune/ptc`.
- Live backups will be under `/srv/backups/dune/live`.
- `scripts/world-capsules.sh inventory` reports current package roots,
  namespaces, NodePorts, PVCs, image tags, and backup buckets.
