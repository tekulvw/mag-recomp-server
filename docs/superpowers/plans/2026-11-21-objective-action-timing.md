# Separate secure and demolition action timing implementation plan

**Goal:** Apply the correct owned-skill/leadership timing slot and authored duration to each admitted objective action.

**Proven defects:** Native SecureAndHold receives DemolitionTime today; retained Sabotage A/B ignores SecureAndHoldTime. Native retake/cancel-retaken selects a base from incoming 0214 kind rather than the admitted target state. Original retail RTTI/array/name/arithmetic proof is retained privately. SecureAndHold is slot15 (+1428); Demolition is slot17 (+1430). Existing fused f32 arithmetic/truncation stays. Actual Advanced Explosives ordinal64 is −0.2 demolition in every faction, with no secure skill row in the retained retail catalog.

## Control/runtime owner

- Add Copy+Default `objective_control::ActionTimingModifiers { secure_and_hold:f32, demolition:f32 }` and a typed interaction method. Keep zero and demolition-only wrappers; existing direct Structural consumers stay demolition-only.
- Select slot15 for newly admitted secure actions, slot17 for admitted arm/defuse. Native secure must preview the request on a clone and choose activate duration for Activating/Retaking, deactivate duration for Deactivating/CancelRetaken; Release has no timing rewrite. Install only after the selected duration validates.
- Apply secure timing to newly admitted legacy Sabotage A/B Activate after its existing duplicate/state/owner/range gates. Zero modifier preserves the exact authored u32 path.
- Preserve existing accepted actor/life deadline, duplicate semantics (legacy ACK vs native rejection), release/cancel behavior, maintenance and separate completion/fuse windows. Unselected invalid values cannot poison unrelated actions or releases.
- Use meaningful REDs for demolition leakage into capture and unequal-base retake/cancel before implementation. Test all four native secure entries, synthetic positive secure timing, real demolition only, invalid input atomicity and unchanged deadlines across duplicate/modifier/life/release changes.

## Root actor routing and integration

- Read both effective values from the same authenticated actor/leadership snapshot in ordinary objective and native Domination event routes. Pass typed timings through provider/runtime while keeping compatibility entry points as needed.
- Keep Acquisition Structural-only inputs and direct demolition/trigger attribution paths on their existing scalar API. Avoid unrelated objective or reward changes.
- Verify both event recorders reject without changing objective/runtime/leadership state. Document exact semantics and preserved windows.

## Private source replay and completion

- Load actual SkillCatalog and owned bit64 through normal reservation admission, never direct modifier injection. Real Sabotage A/B stays3000ms with/without Advanced Explosives; C becomes2800 arm/2000 defuse, fuse45000 and completion20000 stay authored. Use actual native Dom profiles for negative capture/positive Structural cases.
- Synthetic slot15 gives positive secure modifier coverage with unequal37/47 bases and separate actors. Preserve native callback/latch semantics.
- Freeze/hash source and inputs; run affected and final fmt/strictClippy/build/Rust/Python/boundary checks, integrate exact checked source after CI, checkpoint privately and continue core gameplay TODOs. Native game/deployment holds and manual releases remain unchanged; independent-world priorities are not changed.
