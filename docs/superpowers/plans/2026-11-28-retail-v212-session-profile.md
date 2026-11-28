# Retail 2.12 session profile implementation plan

> Implement inline with focused test-first codec/session checks, followed by
> existing affected checks and Linux CI. Workspace instructions authorize source
> PRs into the integration branch and agent merges of passing source PRs.

**Goal:** Connect explicit process protocol1793 to MAPS/MAS/MLS authentication,
character slots, creation and character login, without enabling match protocols.
**Spec:** `docs/superpowers/specs/2026-11-28-retail-v212-protocol-design.md`.
**Architecture:** Retain the existing internal character/account messages and
legacy APIs. Add protocol-specific codec entry points and individually verified
1793 wire-ID mappings. The server routing helper selects1793 only when configured;
unimplemented1793 IDs have no legacy fallback. Unknown native fields retain raw
widths and names, with explicit zero/default local policy for newly made records.

## Constraints and evidence

- Application20231, protocol1793, buildABmagprod493603; existing stable defaults.
- Native constructor/serializer evidence is private, bound to engine SHA256
  705a6ec6a25458b2401b869e75e2904597304e2c06d9534a171a1dcd54aca2b3.
- Verified summary adds2bools, record adds6u16+5u32, create adds1bool after NpId.
  The create flag and record-tail semantics remain unknown; do not invent them.
- Real-client acceptance and qualified2.12catalogs remain separate from synthetic
  codec/session acceptance. Homelab owns rollout, replicas stay0 until handoff.
- No bulk extracted tables, raw dumps, game code or credentials enter Git.
- Do not enable old queue/status/clan layouts under1793. Portal HTTP status/admin
  is separate and stays supported. Releases remain human-owned.

## Tasks

1. Write golden ID/header/slot/record/create tests and strict wrong-profile,
   padding, truncated field and unsupported-ID cases; observe RED.
2. Add explicit codec APIs and the minimal verified ID map; extend versioned
   body fields while preserving existing1740/prototype APIs; observe GREEN.
3. Add synthetic MAPS→MAS create/login/record tests with isolated in-memory/temp
   account storage and an existing injected test committer; observe RED.
4. Route each session through process profile, send all3slots under1793, connect
   verified complete-install patch replies; disable old clan decode under1793.
5. Run focused local checks and fullLinuxCI; publish a focused sourcePR into
   codex/retail-v2.12; merge passing source and coordinate exact rollout limits.
