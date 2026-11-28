# Retail 2.12 server integration

## Intent and constraints

Develop MAG 2.12 server support on `codex/retail-v2.12`, starting from public
main `d02c19cf678e4403baef62ae0afe3e81dc31f592`. The homelab owner deploys a
separate local-only test instance. The recomp owner supplies the client port.
The human decides readiness and performs all releases. Preserve stable defaults,
existing services, accounts, stores and private source/capture boundaries.

## Approach

Use an isolated server instance with real application ID 20231 and an explicit
1793 wire profile once connected. Reusing the existing 1740 codec after changing
only the advertised protocol would misread fields. A shared-process multi-version
router adds early-negotiation complexity that this initial test instance does not
need. This is an implementation choice, not evidence of wire compatibility.

## First bounded increment

Implement a separate offline `MagPatch212Codec`, reusing the existing bounded
MSB-first bit reader/writer. Parse 0x01f5 as a u16 count, that many u32 installed
levels, then two bounded ASCII strings. Reject truncated counts, strings, trailing
bytes, null and oversized payloads. Do not guess the client's version from bytes.
Encode the no-download response as 0x01f6 settings followed by 0x01f7 with empty
file name, zero kind/size and one terminal bit. Keep this API disconnected from
runtime session dispatch until an explicit profile and session tests are added.
No archive serving, account writes or world activation belongs to this increment.

Use fresh synthetic fixtures with literal independently derived bytes. Existing
1740 behavior remains covered by its patch/session tests. Unit checks establish
codec behavior only; no native client acceptance is claimed.

## Evidence and limits

Recomp's exact 2.12 launcher ELF SHA256:
`feb5ed5a3e8f3aab74601ae3edc737e3c010208523d3eb322be997cbe3d79318`.
Its 0x5a834 patch serializer uses u16 count/u32 levels; 0x5a50c is the settings
reader; 0x5a7e4 reads the patch-file fields and one terminal bit. Message IDs,
empty-name terminal semantics and field interpretations also have retained
primary reference source, requiring broader native/runtime corroboration:
[Playforward MapsMessages.cs at f49ae57](https://github.com/Playforward/Plugin.Medius/blob/f49ae57cb6b82d73ebcd5c2ead49028d40b145ef/Plugin.Medius/Extensions/Mag/MapsMessages.cs).
The implementation is independently written using these wire observations;
no decompiled/generated game code or private catalogs enter this repository.
The no-download settings retain the existing local 1740 policy values, not a
claim that those defaults were observed in a live 2.12 exchange.

## Following milestones

1. Connect an explicit 1793 process/session profile before MAS/MLS negotiation,
   qualify MAPS patch completion against the fully installed client, reject
   incomplete installations rather than falsely claiming patch availability.
2. Implement and verify 1793 login, character slots/records, skills and loadouts;
   isolate mutable records until compatibility is proven.
3. Map changed game-type values and queue/status layouts; extract effective
   2.12 item/skill/world catalogs with newest-patch precedence in private tooling.
4. Qualify training then one existing match mode on the local-only deployment.
   Native boot/handoff and libpace integration remain recomp-owned blockers.
5. Complete broader gameplay/regression acceptance before human release review.

## Integration builds and prerelease preparation

The user's follow-up asks for branch prerelease workflow support. Prepare a
separate Release Please config/manifest targeting only `codex/retail-v2.12`,
with prerelease versioning `v212` and a PR-only action (`skip-github-release`).
Use distinct component names ending in `-212` and retain the stable config.
The human merges candidate PRs and creates/releases any tags manually.

A separate manual workflow checks the selected branch commit before publishing
linux/amd64 Medius/gameplay/portal integration images. Use run-specific tags
`v212-<full-sha>-<run-id>-<attempt>` and return image digests in job summaries;
no mutable stable/main/latest aliases, chart publication or release creation.
Read existing main build caches without writing them. Preparing workflow source
is not dispatching a build or cutting a release. Existing local-only deployment
ownership stays with homelab. Its latest user direction is to mimic modes
access and portal status, superseding the earlier LAN-only source restriction.

## Connected profile increment

The second source increment explicitly selects1793 for app20231 and pins that
choice per MAPS/MAS/MLS connection. It maps the24 selected handshake/character
IDs individually, using the existing internal message models while changing the
wire representation. No unsupported1793 ID falls back to an old layout.
The original prototype/1740 APIs and startup defaults remain supported.

Independent bounded native checks on the exact2.12 engine confirmed constructor
immediates (base constructor0x10b3b0), list reader0x70f97c, summary0x70e278,
record0x70e914, create marshal0x710130/0x7101e4 and response0x3d59cc. The list has
no allowed-characters byte; summaries add2bits; records add6u16+5u32; creation
adds1bit after the36-byte NP identity. Unknown field meanings remain unknown.
New local record extensions use explicit zero policy pending native acceptance.

The current synthetic acceptance covers complete preinstalled patch checks,
required signed tickets as well as opt-in development names, all3 selectable
slots, creation, MAS login/data and MLS bind/protocol/keepalive/logout. Old clan
and party handling is disabled under1793; legacy training rewards cannot mutate
1793 records. Queue/status/gameplay remain future
work. Native-client acceptance additionally requires qualified2.12 character
catalogs and recomp boot readiness. Local server deployment remains stopped
until those owners receive an explicit source/config ready handoff.
