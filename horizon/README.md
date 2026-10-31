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
| Character store | `MagCharacterStorePath` |

The prototype loader optionally reads `prototype-character-grants.json` beside the externally supplied loadout-defaults file. This file is not distributed here. The loaders in `Server.Medius/Medius/Mag*Catalog.cs` define the accepted schemas; no copied game catalog is needed to compile or run offline checks. Without the required runtime catalogs, content-dependent features cannot provide the corresponding items, grants, skills or progression. No replacement catalog is silently inserted. Deployment also needs the separately configured gameplay bridge and services appropriate to its selected feature set.

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
DNS overrides change only hostname resolution; they cannot change client ports
or replace compatible certificate templates.

## Licensing

The upstream Horizon MIT license is retained in `LICENSE`. The vendored HighResolutionTimer license is retained in `HighResolutionTimer/LICENSE`. Package dependencies retain their respective licenses. These licenses cover the distributed code; they do not grant rights to game data or third-party runtime material.
