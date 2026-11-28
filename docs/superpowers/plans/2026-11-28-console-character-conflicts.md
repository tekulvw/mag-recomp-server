# Console character conflict implementation plan

1. Add failing synthetic tests for explicitly opted-in conflict creation, stable aliases and owner isolation; run and record the missing behavior.
2. Add durable nullable alias field, JSON validation and PostgreSQL migration/mapping. Extend creation scope with bounded candidate names and enforce live alias uniqueness.
3. Retain verified provider on MAS and enable fallback only for psn. Resolve original aliases only within the authenticated owner and selected character.
4. Extend both-profile wire tests, restart/deleted-slot/bounded-race tests and PostgreSQL race/persistence tests. Run focused checks and complete Linux synthetic server suite plus public-tree checks.
5. Return source and evidence to coordinating owner for review; do not commit, push, merge or publish.

Likely edge cases: identical aliases across different owners; case-only retries; foreign name selection; alias with modified creation fields; malformed NP IDs; maximum-length names; repeated unique-index races; deleted alias reuse; concurrent exact retries and cross-owner creates; legacy stores lacking alias field.

## Review correction wave

Regress JSON and PostgreSQL final restore-name ambiguity and 32-byte moved-aside names. Validate the final name before dry run/apply; refuse without backup or row mutation. Round-trip successful restored records through both wire profiles. Generate the single 16-name pool after validation outside transaction retries; retain real uniqueness-race retry checks and verify source has no RNG call inside the retried operation. Run focused red/green checks, then one complete Linux suite.

## Noncolliding store-name boundary

Regress a 32-byte Sony store request without a foreign holder in JSON and PostgreSQL. Use the same 31-byte assignment and durable alias, verify exact retries/reloads, then authenticate and select the 32-byte alias over the actual login/data wire in both profiles through MLS handoff and assigned-name lists. Keep inbound create CString rejection and legacy store API behavior unchanged.
