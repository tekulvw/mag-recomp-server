# Dual authentication and account linking

The approved change preserves portal authentication and adds opt-in Sony NP ticket authentication. It is source-only work: commit, push and open a PR; do not merge, release, deploy or claim real-console compatibility.

## Architecture

Keep `MAG_PORTAL_TICKETS=off|required` unchanged. Add `dual` requiring the existing portal keys/audience, explicitly configured Sony verification keys and service IDs, and access to the portal PostgreSQL database. No Sony key or MAG retail service ID is enabled by default. Both ticket families use bounded MAG NP message reassembly; recognized formats have independent signature verifiers and never fall back after failure.

The Sony verifier supports only the documented NP 2.1 format, release-network issuer, bounded typed fields, ECDSA/SHA-1 public verification and an exact configured service allowlist. Public format references are implementation references, not evidence that MAG uses a particular key/service/version. Tests generate their own keys and tickets; no real tickets or signing material enter the repository.

In dual mode both providers resolve to a current portal account. Signed portal numeric account IDs resolve through `accounts`; Sony subjects use the stable namespace `256:<unsigned-account-id>` and the `external_identities` table. Display names never link accounts. Existing lowercase character-owner keys remain unchanged. Banned, deleted, unnamed or service accounts cannot acquire console access. Database outages fail closed. Portal-only required mode retains its current database-independent behavior.

## Linking

After a valid unlinked Sony ticket, the server creates a random 128-bit, five-minute proof code, stores only its SHA-256 hash in `console_link_proofs`, and refuses game admission. It returns the code to that authenticated connection using the existing SCERT system-message envelope. Receipt/display of this envelope by stock MAG is explicitly unverified. No code or ticket is logged. A signed-in portal person with a claimed name redeems the code on a CSRF-protected page; the transaction proves possession of the console-delivered code and the portal session. Uniqueness prevents an identity or account being linked twice. Expired/reused/conflicting codes fail. Users can see and unlink their own Sony identity. No administrator or matching-name shortcut grants ownership.

The portal owns the schema and browser changes. The Medius services read account/link rows and issue proof rows through a separately configured secret-file Npgsql connection string. All services must point to the same portal database. Current process-local live-session/replay scope remains explicit, not advertised as distributed coordination. Unlink/ban prevents new console admissions; existing game-session leases are unchanged.

## Shared state and character ownership

Both verified families claim the same canonical local account in the existing live-session registry and enter existing MAS/MLS session issuance. Sony uses a separate bounded replay cache keyed by the authenticated ticket digest and resolved portal account ID; portal nonce rules remain unchanged. NP-ID consistency compares against the verified online identity, while storage still authorizes the canonical owner. New console characters store a canonical owner NP ID after request identity validation, avoiding a storage-schema rewrite and preserving legacy record validation. Actual metadata/header exceptions are not invented: retain zero RSA/NP-reserved-byte, protocol and DNAS checks pending console evidence.

## Required verification and external gates

Cover malformed/unknown/forged/expired/wrong-service tickets; format confusion/no fallback; portal regressions; explicit account lookup; different-name linking; same-name separation; proof expiry/single-use/concurrency; bans/deletion/unlink; cross-provider session ownership; chunk framing and credential redaction. Run existing affected .NET, portal Rust, database and public-boundary checks. Use only a throwaway PostgreSQL instance and synthetic identities.

Track exact retail build/service ticket issuance, MAG issuer/key identity, DNS/reachability, SCERT encryption/header behavior, code display, MAPS/patch/MAS/MLS progression, SVO/TLS, character behavior and match/reconnect acceptance in a console checklist. Native/synthetic acceptance is not console acceptance.

## Alternatives considered

Replacing portal tickets with Sony tickets would discard the working native flow. Matching display names or using generic legacy TicketLogin would not prove ownership. Separate verifiers, a durable provider-qualified mapping and the existing game-session path preserve current behavior with the smallest shared authorization surface.
