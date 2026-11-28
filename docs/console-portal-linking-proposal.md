# Console portal linking proposal

This is the pairing-flow proposal for PR 435. The maintainer confirmed on November 28 that the existing path works for text display. PR 435 implements proof issuance and browser redemption; the complete pairing exchange still needs console acceptance.

## Recommended flow

1. The original game signs into PSN and sends its ordinary Sony NP ticket to our server.
2. After signature/service/time validation, an unlinked identity receives a short-lived pairing code on its own connection. The connection receives no character ownership or game credentials.
3. The player opens the portal on a phone or computer, signs in with Google, opens Console identity, and enters the code. The confirmation identifies the console identity being linked and the portal account receiving it.
4. The database consumes the proof once and uniquely links the verified Sony numeric identity to that account. Matching display names or source IP addresses never establish ownership.
5. The console retries sign-in and follows the normal account/character flow. Cancellation or expiry leaves the account unlinked.

Keep the current contiguous 32-hex-character, 128-bit code initially so the existing browser normalization and hashing contract remain unchanged. A shorter human-readable code would require its own attempt limits and brute-force review. Show a plain short portal URL and text code. The reviewed protocol does not establish QR images or an embedded browser. Keep bearer codes out of logs, browser URLs and persistent plaintext storage.

## Presentation qualification

Use the confirmed existing text-display path for the portal address and pairing code. Verify the actual pairing message stays on screen long enough to enter the code and gives a usable retry/back action. Also verify that automatic retries do not rotate a code the player is still entering; issuance lifetime and retry cadence must be checked together.

If that message is invisible or transient, investigate a distinct `LinkPending` path using the existing NEWS text presentation. This must not reuse the ordinary authenticated or `ContentSent` state: that state currently enables character enumeration. A pending connection may only receive pairing instructions and the minimum verified transport/discovery responses required to reach that display. It must not enumerate or mutate characters, obtain account-login/MLS credentials, join a game, or attach to a portal account before proof redemption. Require a fresh sign-in after browser approval.

Retail launcher authentication precedes universe discovery and patch handling. The later engine NEWS/EULA path is different; prototype NEWS behavior is not proof the retail launcher can display it. Implement that fallback only after an exact-build trace establishes a reachable presentation seam. If neither route works, document that an unmodified client is unsupported rather than infer identity from a name or IP.

## Console-only acceptance

- [ ] Confirm the exact retail build can obtain the expected MAG service ticket.
- [x] Existing path supports text display (maintainer confirmation, November 28).
- [ ] Confirm pairing-text capacity, lifetime, code visibility and retry behavior.
- [ ] Complete phone/browser approval and console reauthentication with differently named PSN/portal accounts.
- [ ] Exercise expiry, cancellation, wrong code, already-linked identities and concurrent attempts.
- [ ] If NEWS is required, prove the exact retail screen is reachable and all pending-link ownership/game operations remain denied.
- [ ] Verify no proof code or ticket appears in operational diagnostics.

No client modification, console run, production deployment or release is performed by this proposal.
