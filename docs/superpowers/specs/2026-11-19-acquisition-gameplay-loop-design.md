# Acquisition gameplay loop

The human authorized autonomous completion of the core Acquisition loop on November 19: capture, unlock an existing Escort, drive it, deliver or destroy it, score, complete the match, and requeue. Preserve all three factions and defender-owned maps. Source integration follows passing affected checks and public CI; releases belong exclusively to the human after live-match review.

Reuse the existing ordered external controller, current/target objective guards, owned ground runtime, ordinary character damage, replication and terminal/requeue paths. Extend them with a concrete isolated Engine transaction. Every effect affecting gameplay is applied to one candidate before its record; publication follows installation. Missing gameplay inputs fail clearly. Configuration and original-source evidence remain external/private.

This builds on current main, including authored trigger boxes and existing all-faction matchmaking. The earlier static-partition source PR is an explicit dependency. Rebuilding native Havok, exact broadphase order and low-bit quaternion equivalence would delay the gameplay loop and are deferred. Cosmetic parity, medals/statistics parity and unsupported mines follow the first playable configuration; unavailable weapons must stay unavailable.

Human-controlled sessions, services, accounts and unrelated work are preserved. No deployment or game launch follows from source checks. Completion claims distinguish synthetic Engine integration from a native live match.
