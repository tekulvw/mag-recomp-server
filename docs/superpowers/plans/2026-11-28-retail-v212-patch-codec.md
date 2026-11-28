# Retail 2.12 patch codec implementation plan

> **For agentic workers:** Use superpowers:executing-plans to implement this
> bounded increment inline. Follow the workspace preference for focused checks.

**Goal:** Add an offline 2.12 patch codec without changing stable runtime routing.
**Architecture:** Separate explicit API, sharing only bounded bit primitives.
**Tech stack:** C# / .NET 10, existing synthetic executable test harness.
**Spec:** `docs/superpowers/specs/2026-11-28-retail-v212-protocol-design.md`.

## Global constraints

- Branch `codex/retail-v2.12`; real app ID 20231; eventual protocol 1793.
- No production/default changes, native launches, deployments or releases here.
- No assets, private catalogs, account data or decompiled code in public source.

## Review focus

- A truncated vector must not shift the build/language string boundaries.
- A complete extra byte must fail even if zero; terminal padding stays zero.
- Wrong wire IDs and legacy packet shapes must not be silently accepted.
- Server replies must not enter the client request parser.
- Shared bit helpers must retain current 1740 behavior.

### Task 1: Strict patch request and terminal completion codec

**Files:** Create `horizon/RT.Models/MAG/MagPatch212Codec.cs` and
`horizon/Tests.MagPlugin/Patch212Checks.cs`; register checks in
`horizon/Tests.MagPlugin/Program.cs`.

**Interfaces:** `MagPatch212Request` holds `uint[] InstalledLevels`,
`string BuildString`, `string Language`. `MagPatch212Codec.TryParseRequest(byte[],
out MagPatch212Request)` returns false with null output on malformed input.
`MagPatch212Codec.NoDownloadReply()` returns the two payloads in wire order.
Runtime dispatch deliberately does not consume these interfaces yet.

- [ ] Write request golden fixtures, truncation/count/string/trailing-byte cases
      and literal settings/empty-terminal expectations; run and observe failure.
- [ ] Implement bounded parsing and split completion encoding.
- [ ] Run `dotnet run --project horizon/Tests.MagPlugin -- patch` and existing
      MAPS patch/session checks. Expected: selected synthetic tests pass.
- [ ] Run `python3 tools/check_public_tree.py`, inspect diff and commit source.
- [ ] Send exact commit, limits and next milestone to recomp/homelab owners.

### Task 2: Branch candidate and integration image workflows

**Files:** Create `.github/workflows/v212-integration-images.yml`,
`.github/workflows/v212-release-please.yml`, `release-please-v212-config.json`,
`.release-please-v212-manifest.json`, `tests/test_v212_workflows.py` and
`docs/retail-v212-integration.md`.

**Interfaces:** Candidate workflow targets `codex/retail-v2.12`, prepares
prerelease PRs only. Manual image workflow consumes its exact `github.sha`,
checks it with reusable `check.yml`, builds the three application contexts and
reports digest references. Neither invokes the release publisher.

- [ ] Add workflow-boundary checks; observe missing-file failure.
- [ ] Implement branch guards, PR-only prerelease config and manual image graph.
- [ ] Run workflow checks, existing main-image checks and actionlint.
- [ ] Document deployment handoff and Release Please human-only lifecycle.
- [ ] Commit and coordinate source pins; no remote dispatch in this task.
