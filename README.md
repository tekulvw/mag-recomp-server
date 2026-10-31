# mag-recomp-server

Replacement server implementation for MAG, with a Rust gameplay engine and a
modified Horizon transport and lobby stack. This is an experimental source
project. It is not an official Sony or Zipper Interactive service.

This repository contains independently written replacement logic and attributed
open-source dependencies. It contains no original game executables, firmware,
assets, extracted game catalogs, captured client packets, accounts, or production
credentials. Operators supply required runtime resources separately.

## Layout

| Path | Contents |
| --- | --- |
| `gameplay/` | Rust gameplay state, protocol codecs, bots, collision and navigation |
| `gameplay/vendor/recastnavigation/` | Pinned Recast/Detour sources and original license |
| `horizon/` | Horizon SCERT/Medius transport, lobby integration and synthetic checks |
| `tools/extraction/`, `tools/collision/`, `tools/navmesh/` | Original-input extractors and evidence-dependent projectors; see [extraction guide](docs/extraction.md) |
| `tests/`, `tools/` | Publication-boundary checks and synthetic input provenance |
| `docs/` | Build instructions and public/private resource boundary |

## Build and test

Install Rust 1.99.0, a C++11 compiler, Python 3.11 or later, and the .NET 10 SDK.
The first build downloads the dependencies named in the manifests.

```sh
python3 -m unittest discover -s tests -v
python3 tools/check_public_tree.py
cargo test --locked --manifest-path gameplay/Cargo.toml
cargo build --locked --manifest-path gameplay/Cargo.toml --bins --examples
dotnet build horizon/Horizon.Server.sln -c Release
dotnet run --project horizon/Tests.MagPlugin/Tests.MagPlugin.csproj -c Release
```

Alternatively, `docker build --target check horizon` compiles the Horizon solution
and runs its offline synthetic checks. Build the gameplay image with
`docker build gameplay`. Neither image packages game catalogs or credentials.
See [BUILDING.md](BUILDING.md) for details.

## Releases and container images

Release Please manages a shared version for the Medius and gameplay images.
Merging its release PR creates a GitHub release and runs checks on the release
tag before publishing separate `linux/amd64` images:

- `ghcr.io/tekulvw/mag-recomp-server-medius`
- `ghcr.io/tekulvw/mag-recomp-server-game`

Stable releases publish full version tags such as `0.1.0`, a moving minor tag
such as `0.1`, and `latest`. Images contain replacement server code and synthetic
defaults; operators still provide their own permitted runtime resources.
See [the release guide](docs/releasing.md) for setup, versioning and recovery.

## Runtime resources and status

The gameplay engine and Horizon exchange authenticated gameplay state through a
JSONL bridge. Their native protocol codecs remain distinct from that bridge.
Existing implementation covers admission, replication, loadouts, damage,
progression, objectives, bot behavior, and collision/navigation processing.
Presence of code is not a claim that every feature or map is complete.

The public tests exercise synthetic inputs. They do not establish real-client
acceptance or parity with a separately configured deployment. This repository
does not include a playable game, a ready-to-use game data pack, an account
database, or a public service endpoint. See the
[resource boundary](docs/resource-boundary.md) and the
[Horizon runtime requirements](horizon/README.md).

## Contributions and license

Read [CONTRIBUTING.md](CONTRIBUTING.md) before adding sources or fixtures. New
project contributions use the [MIT license](LICENSE); separately attributed
dependencies retain their own notices in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
No license in this repository grants rights to the game or its assets.
