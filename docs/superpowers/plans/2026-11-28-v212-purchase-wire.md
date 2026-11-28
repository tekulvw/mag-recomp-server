# Offline 2.12 purchase wire

Scope: the next bounded wire increment under the existing retail-v212 protocol
spec. Preserve the running authentication-only deployment and manual releases.

Exact-engine static evidence proves request/response IDs0268/0269, a plain
NetGameMessage base, a signed16-bit request ordinal, and a response success bit
followed unconditionally by the full1793 record. Native ownership accepts0..159;
wire parsing retains the full signed16-bit range without inventing eligibility.
The current catalog/tier rules remain unqualified and purchases stay disabled.

1. Test invented signed16 boundaries, literal header/body bytes, strict profile
   selection and malformed lengths. Test success/failure response record bits
   against independently constructed synthetic values.
2. Add an explicit MagPurchaseSkill212Request instead of widening the old sbyte
   DTO. Map selected IDs and use the existing1793 record codec for responses.
   Reject the old request DTO under1793 and the new one under older profiles.
3. Keep the new request outside runtime client dispatch. Tests must prove the
   offline parse succeeds while the live client parser refuses it. Bootstrap
   catalog guards and legacy purchase behavior remain covered.
4. Run affected synthetic checks and LinuxCI, then integrate a focused sourcePR
   into codex/retail-v2.12. Do not deploy it or claim native purchase acceptance.
