# Horizon components for mag-recomp-server

This directory contains the C# Medius, DME, NAT and universe-information components, shared protocol libraries, and MAG adapters. It is a source distribution. Game-derived catalogs, account stores, captured packets, certificate templates, private keys, patches, game assets and deployment configuration are supplied separately by an operator who is entitled to use them.

## Build and offline checks

Install the .NET 10 SDK, then run from this directory:

```sh
dotnet restore Horizon.Server.sln
dotnet build Horizon.Server.sln -c Release --no-restore
dotnet run --project Tests.MagPlugin/Tests.MagPlugin.csproj -c Release
```

The last command runs a small executable test harness using synthetic values constructed in memory and temporary files. It checks signed skill fields, message direction, padding and length rejection, application-specific loadout widths, full-record fields, external catalog validation, the generated development RSA pair, and the external certificate requirement. It does not connect to a server, launch a game, or need a game installation. It does not establish retail client compatibility or cover the private capture-based regression suite.

`Server.Test` is a legacy network client, not the offline test harness. Do not run it as a build check. `Server.Unified.Launcher` starts multiple services and is likewise not a test command.

An isolated Docker build can perform the same source checks:

```sh
docker build --target check -t mag-recomp-server-horizon-check .
```

The default Docker target additionally publishes Medius, DME and MUIS into a runtime image. Its build context allowlist admits C# source, project metadata and licenses; no runtime configuration, catalogs or captured fixtures are copied. No Docker build target starts the services. The image entrypoint selects Medius; the DME and MUIS assemblies are also available under `/app/dme` and `/app/muis`. There is no bundled restart supervisor or multi-service deployment script.

The common utility layer uses .NET's `Enumerable.MinBy` and `Task.WaitAsync`. Empty minimum selections still throw, while timeout wrappers follow BCL rules: an already completed task wins even at a zero timeout, a pending task times out immediately at zero, and timeouts do not cancel underlying work. The consecutive grouping helper retains strict aggregate boundaries and oversized singleton groups. Earlier inherited versions cited Stack Overflow and C# Corner snippets; those implementations have been replaced rather than redistributed under an assumed license.

## External runtime inputs

Standalone programs read settings from the configuration directory passed as their first argument; see each program's `CONFIG_FILE` and `DB_CONFIG_FILE` constants. Keep generated settings, database files, logs, account state, certificates and keys outside this source checkout. Defaults may be written when configuration is absent, so inspect and provision configuration before operating a service. The unified launcher also generates configuration and starts services; building it does not make a deployment ready.

When `UsePublicIp` is enabled, set `PublicIpOverride` in server configuration or set the external environment variable `HORIZON_PUBLIC_IP` to an IPv4 address. There is no third-party HTTP address discovery. Missing or invalid explicit configuration fails with a clear error.

The MAG adapter switches in `Server.Medius/Config/ServerSettings.cs` remain opt-in. Select application 20230 or 20231 explicitly. Catalog loaders retain their existing schema checks and use external absolute paths. The principal inputs are:

| Runtime input | Environment variable / setting |
| --- | --- |
| Prototype item bindings | `MAG_ITEM_CATALOG` / `MagItemCatalogPath` |
| Prototype starter loadouts | `MAG_LOADOUT_DEFAULTS` / `MagLoadoutDefaultsPath` |
| Prototype skill catalog | `MAG_SKILL_CATALOG` / `MagSkillCatalogPath` |
| Development progression policy | `MAG_DEVELOPMENT_PROGRESSION` / `MagDevelopmentProgressionPath` |
| Retail armory catalog | `MAG_RETAIL_ARMORY_CATALOG` |
| Retail skill catalog | `MAG_RETAIL_SKILL_CATALOG` |
| Progression catalog | `MAG_PROGRESSION_CATALOG` |
| Training-completion catalog | `MAG_TRAINING_COMPLETION_CATALOG` |
| Optional starting level for new characters | `MAG_STARTING_LEVEL` |
| Optional minimum level after completing Training | `MAG_TRAINING_LEVEL` |
| Character store | `MagCharacterStorePath` |
| Optional hours before an abandoned open match reward row can be compacted (default 24) | `MAG_REWARD_LEDGER_OPEN_MATCH_TTL_HOURS` |

The prototype loader optionally reads `prototype-character-grants.json` beside the externally supplied loadout-defaults file. This file is not distributed here. The loaders in `Server.Medius/Medius/Mag*Catalog.cs` define the accepted schemas; no copied game catalog is needed to compile or run offline checks. Without the required runtime catalogs, content-dependent features cannot provide the corresponding items, grants, skills or progression. No replacement catalog is silently inserted. `MAG_STARTING_LEVEL` starts new characters at that level, and `MAG_TRAINING_LEVEL` raises a character completing Training to at least that level after the Training XP bonus; both grant the level's XP threshold and every gained level's skill points, as if earned. `MAG_TRAINING_LEVEL` applies only to new Training completions (characters that completed Training earlier are unchanged) and never lowers a level or XP. Both require the progression catalog, `MAG_TRAINING_LEVEL` also the Training-completion catalog, and a level the catalogs define; otherwise startup fails naming the variable. Deployment also needs the separately configured gameplay bridge and services appropriate to its selected feature set.

## Reward ledger bounds

The character store (`MagCharacterStorePath`, 1 MiB hard bound) also holds the reward ledger. Each match row (`matchRewardProgress`) is one character's progress in one match: its match XP, sub-point fraction and FRAGO tally. Each receipt (`rewardReceipts`) makes a redelivered reward replay its original result. Gameplay resends every unacknowledged reward each second, so receipts must outlive any redelivery. Action XP is credited to the character when each reward commits; a match row only carries the basis for later rewards in that match, such as the match result bonus.

A row is **finalized** when its match result reward commits. Gameplay sends that reward only to players still in the match at its end. Rows therefore stay **open** when:

- the player leaves before the match result;
- the match is abandoned, its world reset or torn down, or its match profile changed;
- the gameplay worker restarts, which starts a new round with a new match key, or Medius restarts without adopting the world;
- the round never reaches a terminal result;
- the row was written by an older build before match results finalized rows.

Medius cannot tell these apart from a quiet match in progress. The reward match key is a digest that the match terminal notice does not carry. So rows are **dated** instead: each committed reward stamps its row with `lastRewardAt` (Unix seconds). A row from an older build gets the time of the next reward commit, so its TTL starts then; until then it is never compactable. At each reward commit a future date is clamped to the current time. Other saves leave the ledger unchanged.

When a commit would leave the store above 3/4 MiB, compactable rows are evicted with all their receipts, least recently rewarded first, until the store is back at 1/2 MiB. A row is compactable when it is:

- finalized; or
- **abandoned**: open, with no reward for longer than `MAG_REWARD_LEDGER_OPEN_MATCH_TTL_HOURS` (default 24, 1 to 720; otherwise startup fails naming the variable).

Active open rows are never evicted, nor is the row the current commit touches. The eight most recently rewarded compactable rows are kept so that a redelivery after a lost acknowledgement still replays. They are evicted only when the store would otherwise exceed its hard bound. Each compaction logs one line, with finalized and abandoned row counts. If active open rows alone overflow the store, the commit is refused, and the reason gives their count.

An abandoned row is not closed; it is only eligible for eviction. While it is kept, a late reward for it commits normally, continues its match XP and dates it again. After it is evicted, a late reward opens a fresh row and is credited once. That fresh row's match XP and fraction start at zero, so a later match result bonus for that match is computed from the fresh row only. This is never more than the full bonus. Partial progress needs no settlement: its action XP was credited as it was earned, and only a sub-point fraction is dropped. One limit remains, as for finalized rows. A reward that was committed but not acknowledged for longer than the TTL, whose row was then evicted, would be credited again on redelivery. Gameplay holds unacknowledged rewards only in memory and resends them every second, so this is not expected.

## Keys and SCERT certificate templates

No fixed RSA private key is distributed. Settings default to one freshly generated key pair per process, shared within that process. It uses the legacy protocol's 512-bit modulus and exponent 17. This preserves the wire width and existing JSON schema, not modern cryptographic security. A restart changes the default unless an operator supplies a persisted configuration.

For compatible persistent operation, supply keys through the existing settings objects using decimal string properties `n`, `e` and `d`. In particular, coordinate Medius `MPSKey` with DME `MPS.Key`; separate processes have independent development defaults. Configure the appropriate `DefaultKey` for clients and certificate templates. Do not commit serialized generated settings: they contain private key values. Legacy ticket-login account creation now uses a fresh random local password seed instead of a shared password; this field is not part of the ticket packet. Integrations may explicitly supply their own credential policy.

PS3 hello serialization requires external raw certificate-packet templates:

- `HORIZON_SCERT_MAS_CERTIFICATE_PATH`: MAS template, at least `0x1d4` bytes so the existing 64-byte RSA modulus field at offset `0x194` fits.
- `HORIZON_SCERT_MLS_CERTIFICATE_PATH`: MLS template, nonempty and compatible with the selected client. The existing encryption-disable prefix is serialized separately.

The code also accepts explicitly supplied `RT_MSG_SERVER_HELLO.Certificate` / `MLSCert` byte arrays. These are runtime inputs, not published fixtures. Missing templates or insufficient length cause an explicit exception. No zero-filled substitute or vendor certificate is emitted. MAS serialization patches a copy of the supplied template with the selected public modulus; operators remain responsible for a template and keys compatible with their client. PS2 hello serialization retains its original short layout and does not require these templates.

## Client-visible network endpoints

When deploying behind a load balancer, set `MAG_ADVERTISE_IPV4` to its reachable
dotted-decimal IPv4 address and `MAG_AUTH_DNS` to the authentication hostname.
`MAG_AUTH_PORT`, `MAG_LOBBY_PORT` and `MAG_GAME_TCP_PORT` select the ports sent in
MAPS, login and game-join replies. Defaults preserve the local development
endpoints: loopback, `mag.test`, and ports 10475, 10478 and 10473 respectively.
Invalid addresses, hostnames and out-of-range ports fail explicitly.

These settings describe the endpoints seen by clients. Configure MAS/MLS
listener ports separately in `medius.json`. The embedded local DME listens on
TCP 10073 and UDP 10474 on all interfaces; its handshake advertises the selected
IPv4 address and UDP 10474. A load balancer must map the advertised game TCP
port to 10073 and preserve UDP 10474. The Helm chart supplies these mappings.
Each further concurrent human gets the next UDP port; `MAG_DME_UDP_PORT_END`
(and optionally `MAG_DME_HUMAN_CAPACITY`) caps admitted humans at the forwarded
ports, see docs/configured-world-registry.md, "DME port capacity".
DNS overrides change only hostname resolution; they cannot change client ports
or replace compatible certificate templates.

## Portal game tickets

`MAG_PORTAL_TICKETS=required` replaces the development name sign-in on MAPS and MAS with
verified portal game tickets; `MAG_PORTAL_TICKET_KEYS` and `MAG_PORTAL_TICKET_AUDIENCE` supply
the verifying keys and this server's host name. It is `off` by default. See
[docs/portal-ticket.md](../docs/portal-ticket.md#medius-verification-operators).

## Internal admin API

`MAG_ADMIN_API_LISTEN` with `MAG_ADMIN_API_SECRET_FILE` starts an authenticated, cluster-internal
HTTP API for `mag-admin`'s character maintenance in the main Medius process. Both are unset by
default. See [docs/admin-cli.md](../docs/admin-cli.md#admin-api).

## Licensing

The upstream Horizon MIT license is retained in `LICENSE`. The vendored HighResolutionTimer license is retained in `HighResolutionTimer/LICENSE`. Package dependencies retain their respective licenses. These licenses cover the distributed code; they do not grant rights to game data or third-party runtime material.

## Bundled in-game NEWS

MAPS sends the latest release section from each deployed component's changelog
as plain ASCII NEWS, preserving the separate empty EULA response. Medius embeds
`horizon/CHANGELOG.md` when that component changelog exists; Release Please
creates it on the first Medius component release. The legacy root changelog is
intentionally excluded because it mixes server and chart history.

The gameplay image packages its own `CHANGELOG.md` at
`/usr/share/mag-recomp-server/gameplay-CHANGELOG.md`. The Helm chart copies this
file from the **selected gameplay image** into a read-only Medius/MAPS volume.
Changing the gameplay image tag or digest also rolls Medius/MAPS to refresh
that snapshot, even if the Medius image is unchanged. Use immutable version
tags or digests for reproducible image selection. An older gameplay image
without the file contributes no gameplay notes and remains deployable.

For a deployment outside Helm, copy the changelog from the actual selected
worker image and set `MAG_GAMEPLAY_NEWS_PATH` to its absolute, read-only file
path before starting Medius/MAPS. Without this setting, only embedded Medius
notes are available. No notes are fetched from GitHub at runtime; notes are
loaded once per process. The NEWS timestamp comes from the newest included
release date, so rebuilding or deploying a chart alone does not announce a
new server update. If neither component has notes, NEWS shows an availability
message with timestamp zero. English is the currently supported NEWS language.

NEWS removes Markdown links and commit/issue suffixes, normalizes supported
ASCII text, and bounds the combined content to the codec's 4096-byte string
limit, retaining both component headings when either release is long. These
are source and synthetic wire checks; real-client display and refresh behavior
still require an owned in-game check.
