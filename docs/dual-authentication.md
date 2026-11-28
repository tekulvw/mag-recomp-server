# Portal and Sony authentication

Portal authentication remains the supported default. `MAG_PORTAL_TICKETS=off` and
`required` retain their existing behavior and ticket format. `dual` is an opt-in
extension: both signed portal tickets and explicitly trusted Sony NP tickets
resolve to the same portal account, existing lowercase character owner and game
session flow. This is **not yet a claim of real-console compatibility**.

## Enabling dual mode

1. Run the portal normally so SQLx applies migration
   `portal/migrations/0009_external_identities.sql`. Preserve migration 0008,
   which belongs to player reports. Existing Google accounts, devices and
   tickets need no migration or re-enrollment.
2. Configure both MAPS and main Medius with their existing
   `MAG_PORTAL_TICKET_KEYS` and `MAG_PORTAL_TICKET_AUDIENCE`, then set
   `MAG_PORTAL_TICKETS=dual`.
3. Set `MAG_PSN_TICKET_KEYS` to comma-separated
   `eight-lowercase-hex-key-id:base64-DER-SPKI` entries. These must be trusted
   Sony public verification keys using named secp192r1. No key or issuer
   identifier is built in. The public LBP verifier is a format reference,
   not evidence that its key identifies MAG tickets.
4. Set `MAG_PSN_SERVICE_IDS` to an exact comma-separated allowlist of Sony NP
   service IDs, each 1–24 ASCII letters, digits, `_` or `-`. No MAG retail
   service ID is assumed or preconfigured. A product ID and a Medius app ID
   are not NP service IDs.
5. Set `MAG_IDENTITY_DB_CONNECTION_FILE` to an absolute path to a mounted
   secret containing an Npgsql connection string for the **portal** database.
   Use the same database in all services. Configure appropriate TLS verification
   and CA material for the database connection. The file is read once; neither
   its contents nor driver exception details are logged by authentication.
   Missing schema or connectivity refuses startup. Runtime database outages
   refuse admission, including portal admission in dual mode.

The Medius database role needs SELECT on `accounts` and `external_identities`,
and SELECT/INSERT/DELETE on `console_link_proofs`; it does not need to modify
accounts or identity links. The portal owns link redemption and unlinking.
Portal-only `required` mode retains database-independent ticket verification.

The console also needs reachable MAPS/MAS/MLS addresses. Dual mode does not
change DNS, SCERT encryption, installed game files, patch policies, SVO URLs,
DNAS handling or TLS trust.

## Linking accounts

An unlinked Sony ticket must first pass signature, network, service, identity
and time checks. Its numeric subject is namespaced as `256:<unsigned account
ID>`; neither its online ID nor an equal portal name grants character ownership.
The server creates a random 128-bit code, stores only SHA-256 of its lowercase
32-hex-character text, and sends the code only to that connection using the
existing SCERT system-message envelope. Game admission is refused until linked.

The player signs into the normal portal and opens **Your account → Console identity** (`/account/console`). They enter the received code. The CSRF-protected
transaction locks and rechecks their current account, consumes one unexpired
proof and links the verified Sony subject. An identity can belong to only one
portal account; each portal account can have one Sony identity. Conflicts roll
back without consuming the proof. Names never automatically link accounts.
Banned, unnamed, deleted and service accounts cannot acquire console access.

Codes last at most five minutes and never beyond the Sony ticket expiry.
A new proof for the same subject invalidates its previous code. The shared
pending-proof table is bounded at 10,000; full storage refuses issuance.
Codes/tickets are not included in authentication logs or browser responses.
The browser accepts uppercase hex and outer whitespace, normalizing before
hashing. It does not accept ticket uploads or ask for Sony credentials.

Players can see and unlink only their own Sony identity. Account deletion
cascades identity links; the existing name reservation remains. Bans,
deletion and unlink prevent new Sony admissions. Existing game-session leases
are unchanged, so unlink is not an immediate kick/revocation operation.

**Stock MAG display of the system message is unverified.** If the original
client does not display it, a supported delivery mechanism must be established
before stock-console linking is usable. Do not ask players to retrieve/log raw
PSN tickets as a workaround.

## Shared ownership and limitations

Both providers resolve to the current portal account ID and canonical lowercase
owner. Existing signed portal service accounts remain eligible for portal admission;
only person accounts may acquire or use a Sony link. A signed portal name must still match that account. Sony online IDs can
differ from the portal name. Character creation checks the request against the
verified online ID and then preserves the existing canonical-owner NP-ID storage
contract. No character store or owner names are renamed. Hardware must still
confirm that console character screens accept these returned canonical NP IDs.

Dual-mode MAG plugin requests execute in a bounded off-tick worker queue: four
callbacks at a time, 128 admitted requests total and eight per channel including
running work. Each channel is processed in order, closed channels are skipped,
and replies are published only to the still-active connection with its immutable
application binding. Initial activity precedes published session state, and activity
timestamps are atomic and monotonic across tick/worker threads. Capacity exhaustion
refuses that connection; database waits do not block shared world/echo processing.

Both providers claim the same process-local live-account registry. Portal
nonce rules remain unchanged. Sony replay identities hash the authenticated
signed prefix, excluding the malleable ECDSA signature. Each ticket permits
32 uses per service, with bounded cache size and per-account counts. These are
conservative synthetic-tested limits pending observed console reconnect flow.
They are not cluster-wide leases/replay protection: existing MAPS/MAS service
separation and process-local state remain. Independent replicas do not enforce
a global one-session limit.

The initial Sony parser accepts only the documented 248-byte NP 2.1 structure,
release issuer 256, empty-entitlement fields and canonical padded DER ECDSA
signature. It validates exact nested types/lengths, configured key and service,
nonzero unsigned account ID, bounded online ID, issue/expiry milliseconds and
at most a 12-hour lifetime. Unknown versions, entitlement variants and trailing
data fail closed. No failed ticket falls through to plaintext development auth
or another issuer. This intentionally narrow format needs MAG-specific evidence
before expansion.

## Offline verification

All tests use invented accounts and generated keys/tickets. No Sony signing
secrets, original game files or real tickets are required.

- `dotnet run --project horizon/Tests.MagPlugin/Tests.MagPlugin.csproj -- 'dual authentication' 'Sony NP'`
  checks parsing/signatures, strict configuration, canonical ownership, shared
  sessions, differing-name character creation, prototype/retail envelopes and
  code redaction. Linux is required for the complete existing durable-storage
  suite.
- Set `MAG_TEST_POSTGRES` to an Npgsql connection string for a **throwaway**
  PostgreSQL server with CREATE DATABASE rights, and
  `MAG_REQUIRE_POSTGRES_CHECKS=1`, to require the database checks. Tests create
  and drop their own databases. The .NET fixture is byte-identical to portal
  migration 0009; Python enforces that contract.
- With `DATABASE_URL` pointing to a throwaway PostgreSQL server, run
  `cargo test --locked --manifest-path portal/Cargo.toml`. Console tests cover
  proof expiry/reuse/conflicts, concurrency, lock-wait expiry and bans,
  deletion, ownership, CSRF and the browser routes; existing OIDC/device/ticket
  regressions remain in the suite.
- Run Rust formatting/clippy, Python checks and
  `python3 tools/check_public_tree.py` before publication.

## Console acceptance checklist

These are outstanding, not synthetic passes:

- [ ] Establish exact retail build, region and NP service ID; prove PSN still
      issues that title's ticket. General PSN sign-in does not prove issuance.
- [ ] Establish actual MAG ticket version/size, entitlement variant, issuer/key
      ID, public-key trust and service binding. Configure trust only after this.
- [ ] Verify hostname/DNS redirection and reachable MAPS/MAS/MLS endpoints from
      an unmodified console/client; native archive overrides are not proof.
- [ ] Verify SCERT handshake/encryption, plugin RSA header words, NP-ID
      metadata, protocol version and DNAS fields. Existing zero-header and
      metadata/version checks remain; do not loosen them without evidence.
- [ ] Verify the pairing system message is visible and usable, complete browser
      linking, then retry authentication. Qualify a different delivery adapter
      if this stock client does not display the existing message.
- [ ] Verify launcher MAPS authentication, universe and patch progression,
      engine MAS authentication, existing/new character selection and canonical
      NP-ID behavior for different portal/PSN names.
- [ ] Verify access/session credential handoff to MLS, match join, reconnect and
      ticket renewal/reuse with both native and console clients; test linked
      account exclusivity within the supported service topology.
- [ ] Determine whether this supported path requires SVO/statistics. If it
      does, qualify its ticket verification and actual PS3 TLS/trust behavior.
- [ ] Verify bans/unlink/deletion on the next sign-in and absence of bearer
      credentials in operational diagnostics.
- [ ] Qualify retail 2.12 and other regions independently; v1.0 framing evidence
      does not establish those profiles.

No automatic merge, deployment, release or release publication is part of this
change. Keep dual mode disabled until operator trust configuration and the
required console gates have been reviewed.
