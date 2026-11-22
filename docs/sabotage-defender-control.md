# Sabotage A/B defender control

A configured Sabotage capture objective with `can_deactivate: true` accepts the
owner team's normal `03ea` Deactivate request. Previously the legacy capture
receiver rejected every owner-team interaction. The capture adapter reuses the
common SecureHold state machine without enabling Domination's script scheduler
or changing the objective catalog format.

| Current state | Actor/action | Entered state | Timed result |
| --- | --- | --- | --- |
| Deactivated | Attacker Activate | Activating | Activated, then Completed |
| Activated | Defender Deactivate | Deactivating | Deactivated |
| Completed | Defender Deactivate | Retaking | Retaken, then Deactivated |
| Retaken | Attacker Activate | CancelRetaken | Completed |

Retaking uses the configured activation hold; Deactivating and CancelRetaken
use the configured deactivation hold. Existing effective SecureHold modifiers
are captured when the action starts. A duplicate from the same current actor,
life and selected trigger acknowledges the accepted hold without restarting it.
Requests retain the existing on-foot, match, target, trigger and range checks.
A selected-trigger mismatch or disabled deactivation is refused.

Release, lost contact or loss of the holding actor's current life restores
Activating to Deactivated, Deactivating to Activated, Retaking to Completed,
and CancelRetaken to Retaken. Release may occur outside contact. The restored
Activated or Retaken window keeps its original absolute deadline. Deactivating
uses its own hold deadline while active; an already-expired restored window
becomes due on its next update. Accepted deadlines retain the legacy bounded
late-tick catch-up behavior. Packet and tick ordering remains the existing
recorded ordering; no additional simultaneous-event priority is introduced.

Both A and B must be **currently Completed** for the existing progression to
lock them. Deactivating, Retaking and Retaken do not satisfy that condition.
After the pair locks, further activation/deactivation is refused. The existing
satellite/proxy completion, delayed C unlock, score/bleed and demolition paths
remain in use. Existing forward activation/completion rewards and capture
attribution are retained; this change adds no new stat or XP callbacks for the
new reversible edges.

The adapter is attached only to configured mode-0 capture objectives beginning
in an existing stable Inactive, Deactivated, Completed or Locked state. Other
modes and transient-initial-state legacy configurations retain their existing
receiver. An absent objective catalog does not enable this behavior. The
retained Sabotage catalog explicitly uses a development point-distance policy;
this change preserves it and does not qualify new native trigger geometry.

Interaction changes stay in the cloned objective-world candidate until the
existing event record succeeds. Timed changes use the same record-before-install
boundary. Peer tick publications use the existing bounded reliable FIFO, with
recipient sequence allocation when it drains. A full response byte budget can
defer a queued frame without discarding or overtaking older frames. Existing
recipient-backlog lifecycle policy is unchanged.

The original qualification covers the common retail SecureHold entries and the
captured Syr Darya A/B property/script binding: three-second entering holds,
twenty-second completion windows, pair locking, proxy completion and the
five-second delayed C callback. Public regression fixtures are independently
authored and exercise real `03ea` ACK/NAK/exit behavior, all four release paths,
contact/life loss, duplicate timers, contest/lock progression, record refusal
and shared FIFO deferral. Original assets and script/native source are not
included in the public repository.

Physical forward insertion remains a separate unsupported scope. The retained
original Sabotage forward links identify inactive type-5 LALO points. Completed
forward-spawn proxies do not establish a supported physical LALO receiver, and
this increment does not substitute ground spawns or claim live-client,
multi-human or all-map acceptance.
