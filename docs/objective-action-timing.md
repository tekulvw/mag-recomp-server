# Objective action timing

Capture and demolition use separate effective character modifiers.
`kCharModSecureAndHoldTime` controls supported secure holds;
`kCharModDemolitionTime` controls arm and defuse. Both values come from one
authenticated actor/leadership snapshot. Existing owned Advanced Explosives
reduces demolition time by 20%; it does not reduce capture time.

The original retail modifier table assigns SecureAndHold to slot 15 and
Demolition to slot 17. The native state adapters choose their authored duration
before applying the shared fused single-precision calculation and truncation.
The replacement now makes the same selection from the admitted entering state:

| Entering secure state | Authored base | Modifier |
| --- | --- | --- |
| Activating | Activation duration | SecureAndHold |
| Retaking | Activation duration | SecureAndHold |
| Deactivating | Deactivation duration | SecureAndHold |
| CancelRetaken | Deactivation duration | SecureAndHold |
| Arm or defuse | Matching demolition duration | Demolition |

An incoming request alone does not identify the base duration. A defender's
Deactivate request can enter Retaking, and an attacker's Activate request can
enter CancelRetaken. Native secure requests are previewed on a candidate before
validating the selected timing value, so invalid timing cannot leave a pending
transition behind.

Legacy Sabotage A/B applies SecureAndHold timing when a new attacker hold is
admitted. Existing duplicate acknowledgements retain the original deadline.
Native secure duplicates keep their existing refusal behavior. Structural-only
Acquisition and the scalar compatibility APIs remain demolition-only; new typed
consumers use `ActionTimingModifiers { secure_and_hold, demolition }`.

The accepted actor/life action keeps its deadline across later modifier changes.
Release and cancellation retain their ownership and state checks. Irrelevant
timing values cannot block release, accepted legacy duplicates or an action of
the other kind. Authored secure completion windows, maintenance intervals and
demolition fuses are separate and are not shortened by either modifier.

The retained original skill catalog contains no SecureAndHold skill row.
Synthetic inputs exercise that positive slot; original owned Advanced
Explosives data verifies unchanged capture and shortened demolition through
normal admission. Source tests and scripted replays do not establish native
live-match acceptance. Releases remain manual after the maintainer's review.
