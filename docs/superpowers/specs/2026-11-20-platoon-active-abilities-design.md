# Platoon command authority and effects

The next gameplay increment enables the retail platoon leadership layer and its authored commands in supported Acquisition and Domination source profiles. It builds on the squad runtime, recorded effect transaction, real collision providers, and ordinary combat receivers. User direction is autonomous implementation and source integration after affected checks; live-match review and releases remain manual.

## Architecture and choices

Extend the existing native held-role model rather than introducing development role grants. Track native squad and platoon scopes separately, with round-owned cooldowns and native type-10 AbilityData publication. A leader is authorized from authenticated current reservation identity, never client-provided role bits. Platoon promotion runs before squad vacancy repair and may promote a sitting squad leader.

Reuse the current scheduled effect transaction and normal damage consumers. Add typed effect rules extracted by neutral tools from user-supplied original inputs. Each effect requires retail qualification of timing, targeting, geometry, damage and failure behavior before implementation. Keep original catalogs, native disassembly and receipts private. Public source contains handwritten replacement logic and synthetic tests only.

Alternatives considered: a separate platoon engine would duplicate cooldown/rollback/combat logic; generic explosions would erase distinct retail targeting and receiver rules. Extending the existing engine keeps the proven transaction boundary and preserves those differences. This architecture is the implementer's choice within the requested scope.

## Requirements

- Resolve native platoon election eligibility, scoring, slot ordering, promotion, removal and replacement from retail. Debug symbols may guide discovery; retail verifies decisions.
- Preserve squad behavior, schema compatibility where meaningful, and fail clearly when a required new external input is missing.
- Authenticate exact held scope and current identity; do not grant OIC authority or persistent character progression.
- Execute permitted precision, strafe, gas and artillery effects only for the actual mode/side/faction lists; preserve shared and unit cooldowns, anti-air state, and tactical-asset vulnerability.
- Record admitted commands and commit effects atomically. Failed records/queries retain the previous candidate; reliable queue pressure defers publication without consuming it.
- Publish leader, active state and current AbilityData for late joins and role changes. Retain per-scope state across leadership changes, clear on round reset.
- Preserve C4-only objective/emplacement immunity established in the previous increment.

## Bounds and acceptance

Fresh main baseline is public 8640dc3. No game launch, deployment, release or released submodule pin change is included. Native client presentation and live-match acceptance remain separate. Source acceptance requires focused red/green regressions, full affected Rust/Python suites, strict Clippy/fmt, public boundary checks, and checked public PR integration. Unresolved native behavior must remain explicit and cannot be substituted with invented gameplay.
