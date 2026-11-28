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
codec fallback is allowed for them. A qualified2.12 character catalog set and
client boot readiness are still required before any live acceptance run.

The homelab chat owns the separate `mag-212` test deployment and its own storage,
private DNS, portal audience/status and network policy matching modes. Existing
accounts, services and mutable stores remain separate. Recomp owns native
launcher/engine readiness. Coordinate credentials through the existing secret
workflow, never source commits or public PR bodies.

The planned first native test uses app20231/protocol1793 and fresh isolated
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

```sh
dotnet run --project horizon/Tests.MagPlugin -- patch
python3 -m unittest discover -s tests -p test_v212_workflows.py
NODE_PATH=/path/to/release-please-17.6.0/node_modules node tests/release-routing-v212.cjs
python3 tools/check_public_tree.py
```

The complete Medius suite includes Linux-only durable store operations; run it
on the existing Linux CI runner or the Dockerfile's check target. macOS codec
checks are narrower evidence and do not replace the Linux suite.
