# Retail projectile combat continuation

**Goal:** Complete ordinary retail fourth-slot frag behavior and add the
source-qualified Ground vehicle channel to equipped hospital rocket terminals.

**Baseline:** Fresh main `983dd27dc6304291243695b50f72c29923a00542` after the
on-foot rocket integration. Both SDK mains were refreshed; unrelated state and
manual-only releases remain preserved. The user requests autonomous continuation.

## Interfaces and scope

Ordinary frag storage is four elements, with explicit configured-profile admission
and blast helpers. Public legacy wrappers retain prototype behavior. Retail
supports four slots; prototype supports three. Cleanup, mixed owner updates and
shared rocket/frag conflicts must retain the same wire identity and FIFO rules.
The exact native producer wrap cadence is not claimed.

Optional `core.hospital_rocket_ground` requires the equipped rocket provider. Its
strict external catalog binds original ELF/Main, the current dynamic catalog,
Ground policy, six source roles and native chassis damage-area thresholds. The
maintained original-input projector reproduces these values. The classifier uses
the current model pose and blast origin, not an offset root pose or generic area0.

The Ground-only planner takes the vehicle dose, tests current solid-body overlap
and one layer24 material ray to the vehicle anchor, then applies native locations
1–6, type10, zero redux and existing friendly/armor rules. Keep the Ground source
word at its established invalid sentinel; projectile identity remains in audit.
Helo, objectives, blocked-ray fallback and additional direct-impact damage stay
separate. Ordinary later Ground critical/destruction/hulk/respawn handling remains
the lifecycle consumer.

`RocketEffects` stages Character and optional Ground candidates. Preflight reserves
Character packets, one Ground packet when present and the terminal together.
One successful record precedes both installs and reliable publication. Queue
pressure defers all effects while permitting ordinary drain; recorder refusal
leaves both health states, the flight and queues unchanged.

## Work and evidence

- Frag owner: explicit profile helpers, storage and lifecycle regressions.
- Bounds owner: maintained projector, typed source catalog and native classifier.
- Ground owner: immutable current-vehicle planner, armor and packet wrappers,
  combined transaction tests and original-data replay.
- Root: central defaults/config/profile readiness, combined terminal commit,
  full checks and focused source integration, then the next core TODO.

Observe behavioral RED/GREEN for slot3 launch/fuse/damage/removal/reuse and prototype
rejection; both conflict directions and mixed owner fields; live model pose,
duplicate roots/phantoms, armor/cover/sentinel; combined success, record refusal
and tight-capacity retry after normal drain. Validate both six-profile startup
scopes and current original-data Ground contacts. Source replay and native-client
live-match acceptance remain distinct.
