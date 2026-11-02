# Bundled Server NEWS Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans inline. The user explicitly requested implementation of the selected bundling approach; proceed without another proposal gate.

**Goal:** Populate MAG NEWS with the latest gameplay and Medius release notes shipped in the selected server images, excluding chart releases and network refreshes.

**Architecture:** Embed the Medius component changelog into its assembly. Package the gameplay changelog in its own image and copy it into a read-only NEWS volume with a Helm init container using the same gameplay image pin as the gameplay Deployment. Medius formats those two immutable inputs into bounded ASCII NEWS and a stable release-date timestamp.

**Tech Stack:** .NET 10, Docker, Helm, existing C# synthetic harness and Python unittest.

**Spec:** User request in this chat, November 2, 2026: gameplay/Medius changelogs, bundled with deployed server, chart-only releases excluded.

## Global Constraints

- Preserve independent component release routing and manual-only releases.
- No GitHub polling, game launches, deployment, accounts, private resources or unrelated source adoption.
- Preserve existing NEWS/EULA transaction IDs, direction, completion and authentication checks.
- Only ASCII text fitting the existing 4096-byte protocol string bound is sent.
- Old gameplay images without a bundled changelog remain deployable and contribute no gameplay notes.

## Review Focus

- Independent gameplay upgrades must also roll Medius so its pinned notes refresh.
- Legacy gameplay images may lack the notes file; do not fail startup or fabricate notes.
- Large notes must retain both component headings and fit the wire bound.
- Markdown links, Unicode and control bytes must not break NEWS serialization.
- Chart-only changes must not change the selected note inputs or timestamp.

### Task 1: Bundle, format and serve deployed component NEWS

**Files:**
- Create: `horizon/Server.Medius/Medius/MagServerNews.cs`
- Create: `horizon/Tests.MagPlugin/ServerNewsChecks.cs`
- Modify: `horizon/Tests.MagPlugin/Program.cs`, `horizon/Server.Medius/Server.Medius.csproj`, `horizon/Server.Medius/Medius/MagMapsSession.cs`, `horizon/.dockerignore`
- Modify: `gameplay/Dockerfile`, `gameplay/.dockerignore`
- Modify: `charts/mag-recomp-server/templates/deployments.yaml`, `tests/test_helm_chart.py`
- Modify: `README.md`, `horizon/README.md`, `charts/mag-recomp-server/README.md`

**Interfaces:**
- `MagServerNews.FromChangelogs(string medius, string gameplay)` produces immutable `Content` and `Timestamp`; `MagServerNews.Current` loads deployment inputs once per process.
- Gameplay image exposes `/usr/share/mag-recomp-server/gameplay-CHANGELOG.md`.
- Helm copies that file to `/news/gameplay-CHANGELOG.md`, supplied through `MAG_GAMEPLAY_NEWS_PATH` to Medius/MAPS.

- [x] Run the existing Horizon and Python baselines.
- [x] Write C# checks for latest releases only, both component headings, stable date, no GitHub metadata/Unicode/control characters, overflow, absent notes, and actual MAPS NEWS/EULA wire responses for both application IDs. Write a Helm check that executes the rendered copy script against synthetic files and verifies image-pin matching, legacy absence, read-only consumption and rollout on gameplay-pin change.
- [x] Run the new checks and observe failure for missing NEWS behavior.
- [x] Implement immutable changelog formatting, embedded Medius source and protocol integration; package gameplay notes and add deployment copy/rollout wiring. Do not copy the legacy root changelog because it includes chart changes.
- [x] Run the full offline Horizon harness, Python suite, Helm lint, image publish build and public-tree guard; report any unavailable checks precisely.
- [x] Review the focused diff, correct material issues and leave a source commit on `codex/bundled-server-news` for review. No release, deployment or game run.

## Verification result

The initial Python baseline passed125 tests. The original Horizon Docker baseline
was blocked before compilation by the full local Docker VM disk; a disposable
memory-backed build compiled the source. After the user reclaimed space, the
normal Docker check and both complete runtime image builds passed. Final checks:
25 Horizon synthetic checks,126 Python tests, Helm strict lint, the Release Please
component routing fixture,36 PR-title tests, and the public-tree guard. The added
wire regression first failed on empty NEWS and the Helm regression first failed
on the missing copy container. A read-only review found no actionable findings.

Notes are selected from each deployed component image rather than a source-tree
snapshot of both components. Legacy root notes are excluded because they mix
chart and application history. No service/game launch, deployment or release was
performed; real-client display and refresh behavior remain unverified.
