# MAG 2.12 integration builds

The integration branch is `codex/retail-v2.12`. It starts from current main;
source increments go through focused PRs into that branch. Main and released
versions retain their existing behavior until the human approves the switch.

## Candidate versions

`v212-release-please.yml` prepares prerelease PRs against the integration branch.
It uses separate config/manifest files and component names ending in `-212`,
with the `v212` prerelease identifier. This is a server component version, not
MAG's game version. The candidate action has `skip-github-release: true`:
it creates no release or Git tag, and never merges its own PR.

The human merges release candidate PRs and manually creates/publishes any
prerelease tags when wanted. Without human-created tags, Release Please may
continue accumulating commits in a candidate rather than advance a released
sequence. Test image publishing does not depend on merging a candidate PR.
Do not merge the candidate config, prerelease manifest/version bumps or branch
workflow routing blindly into main during the eventual transition.

## Integration images

Use the existing **Main container images** workflow, select
`codex/retail-v2.12`, and run it manually. Its integration-only job calls
`v212-integration-images.yml`; the main jobs remain guarded to main. Reusing the
existing entry point allows dispatch while the new workflow exists only on the
integration branch. Equivalent command after branch publication:

```sh
gh workflow run main-images.yml --ref codex/retail-v2.12
```

The workflow checks the exact selected commit, then builds linux/amd64 Medius,
gameplay and portal images. Tags are:

```text
ghcr.io/<owner>/mag-recomp-server-<medius|game|portal>:v212-<full-sha>-<run-id>-<attempt>
```

Each component job reports its `image@sha256:...` reference. Homelab must pin
those digests and record the source SHA and workflow run for each rollout.
Run-specific tags avoid collisions between reruns; digest references establish
immutable deployed bytes. This path does not update main/latest/stable aliases,
write main build caches, create releases or publish a chart. Passing build
checks does not establish native 2.12 compatibility.

## Current readiness and deployment handoff

The current source adds an explicitly configured protocol1793 session path for
MAPS patch completion, authentication, character slots/creation, MAS character
login/data, and the MLS protocol/keepalive/logout handshake. Each connection pins
its protocol before its initial reply. The legacy application APIs/defaults stay
unchanged. The selected IDs and changed character-field widths were checked
against the exact2.12 engine; the source includes only independently written
code and selected protocol constants, with raw analysis kept private.

Synthetic checks cover both required signed portal tickets and explicit local
development names, three empty/unlocked slots, named-slot creation, login,
record extensions, wrong IDs/profiles, truncated fields, and profile pinning
through config reload. New record extension values default to zero as explicit
local policy; their meanings and accepted native defaults remain unknown.

**This is not native-client acceptance or a gameplay activation contract.**
Queue/status/clan/party wire formats and training rewards remain unqualified under1793, and no old
codec fallback is allowed for them. The explicit bootstrap catalog profile below
supports the first authentication-only test; client boot readiness and the
deployment owner's ready handoff are still required before live acceptance.

## Authentication-only character catalogs

Protocol 1793 startup requires `MAG_CHARACTER_BOOTSTRAP_PROFILE`. The maintained
extractor reads the exact supported engine and three effective resource files:

```sh
python3 tools/extraction/extract-retail-v212-bootstrap.py \
  --engine /external/input/engine.elf \
  --headers /external/input/data_scripts_headers.zrb \
  --classes /external/input/character_class.zrb \
  --equipment /external/input/equipment_headers.zrb \
  --output-dir /external/private/new-bootstrap
```

The output directory must be new. All source SHA256 values are checked before
writing; foreign inputs and output overwrites fail. The bounded reflective
reader retains unknown layouts as opaque values, and projection rejects opaque
progression fields. No original inputs or generated catalogs belong in public
Git, CI artifacts or container layers. Public tests use invented resources.

Mount the three generated files as siblings and set absolute paths:

```text
MAG_CHARACTER_BOOTSTRAP_PROFILE=/data/retail-v212-bootstrap-profile.json
MAG_RETAIL_ARMORY_CATALOG=/data/retail-v212-bootstrap-armory.json
MAG_PROGRESSION_CATALOG=/data/retail-v212-bootstrap-progression.json
MAG_LOCAL_DME=0
```

The profile binds app 20231/protocol 1793, exact engine/resource identities and
both catalog content hashes. Startup checks the same bytes supplied to the
existing Armory/progression consumers before opening the database or listeners.
It rejects a missing profile, altered catalogs, a gameplay DME or world registry,
legacy skill/training catalogs, training-level overrides and respec overrides.
Omit `MAG_WORLD_REGISTRY`, `MAG_RETAIL_SKILL_CATALOG`,
`MAG_TRAINING_COMPLETION_CATALOG`, `MAG_TRAINING_LEVEL`,
`MAG_RESPEC_POINTS_FLOOR` and `MAG_RESPEC_FEE_CAP`. Omit
`MAG_STARTING_LEVEL` or set it to 1. Keep gameplay Deployments at zero replicas.

The selected **operator policy** starts at level 1 with zero additional skill or
leadership points, three authored starter inventories and two cleared wire
inventories per faction, update IDs 1, and ownership of the starter-item union.
There are no purchased-skill grants. Actual authored level grants still apply;
zero additional points does not mean zero total points. No old skill Tiers or
training catalog is synthesized. The private manifest retains static diagnostic
qualification and `native_acceptance: false`, including residual uncertainty
about live item vectors, native identity canonicalization and postload helpers.
These offline checks do not establish native creation/login acceptance.

The deployment owner can package only these three JSON files using the existing
private data-bundle format. Pin its archive digest, member digests and immutable
storage version alongside the image/source pins in the private rollout receipt.

The homelab chat owns the separate `mag-212` test deployment and its own storage,
private DNS, portal audience/status and network policy matching modes. Existing
accounts, services and mutable stores remain separate. Recomp owns native
launcher/engine readiness. Coordinate credentials through the existing secret
workflow, never source commits or public PR bodies.

The planned first native test uses app 20231/protocol 1793 and fresh isolated
test identity selected in private coordination. The client owner has qualified the original2.12 local-name
setter and no-ticket path. Only the mag-212 instance may use the existing explicit
`MAG_PORTAL_TICKETS=off` development-name policy for that test; preserve every
existing row and all other worlds' auth. Portal staff status/admin HMAC remains
independent. Restore required tickets once the2.12 client ticket hook is qualified.
No SVO endpoint is provisioned or inferred from the portal URL.

The deployment owner supplies exact endpoint/secret references and must await
an explicit ready handoff. Use the deployment owner's verified private host/address, with MAPS bootstrap
TCP10073 (not the internal container10471), MAS10475, MLS10478 and MPS10477;
noMAPSUDP. The initial gameplayTCP10473/UDP pool remains unqualified by login
tests. Exact addresses, storage and identity details belong in private receipts. Mode IDs and effective2.12 item/skill/world
catalogs must be verified before gameplay. The first live acceptance target is
launcher boot, character creation and login using the isolated test identity.

## Offline checks

The explicit 1793 codec also supports offline purchase-message inspection:
`MagPurchaseSkill212Request` uses wire ID `0268` and a signed 16-bit ordinal,
without the common RSA/transaction header. `MagPurchaseSkillResponse` uses
`0269`, one success bit and the full 1793 record unconditionally, including on
failure. Use `TryParseForProtocol` / `SerializeForProtocol` with app 20231 and
protocol 1793. The older request DTO and signed-byte format remain distinct.

**Live purchase dispatch remains disabled.** `TryParseClientForProtocol` refuses
the new request, and the bootstrap profile still rejects legacy skill inputs.
The wire retains its full signed range for inspection; it does not authorize an
ordinal. Native ownership supports 0..159, while complete purchase authorization,
equipment rewards, default grants and native purchase acceptance remain separate
work. No legacy Tiers table is substituted for the changed 2.12 resource fields.

```sh
dotnet run --project horizon/Tests.MagPlugin -- patch
python3 -m unittest discover -s tests -p test_v212_workflows.py
NODE_PATH=/path/to/release-please-17.6.0/node_modules node tests/release-routing-v212.cjs
python3 tools/check_public_tree.py
```

The complete Medius suite includes Linux-only durable store operations; run it
on the existing Linux CI runner or the Dockerfile's check target. macOS codec
checks are narrower evidence and do not replace the Linux suite.
