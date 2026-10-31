# Building the replacement server

## Gameplay engine

Use Rust 1.99.0, a C++11 compiler (`c++`, or set `CXX`) and `ar`. The native build
compiles the vendored Recast/Detour sources; it does not extract game resources.

```sh
cargo fmt --manifest-path gameplay/Cargo.toml --all -- --check
cargo clippy --locked --manifest-path gameplay/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path gameplay/Cargo.toml
cargo build --locked --manifest-path gameplay/Cargo.toml --bins --examples
```

The main executable is `mag-gameplay`; additional binaries provide collision and
navigation tooling. Use the argument validation in `gameplay/src/main.rs` for
the current catalog options. A runtime needs item, spawn and other required
catalog files, a new writable evidence-log path, and correctly configured
transport integration. Public synthetic inputs demonstrate schema and codec
behavior; they are not game-compatible catalogs. Missing required resources
cause explicit startup errors.

`EngineResources` is the library integration boundary for loaded resources.
Convenience constructors that expect development catalogs cannot run until
those resources are supplied. Do not use placeholder worlds to conceal missing
runtime inputs.

Map positions come from supplied catalogs. Legacy map-specific placement
constructors are absent from the public API. Bunker mounting, dismounting and
passenger spawning additionally require a validated `BunkerExitPolicy`, supplied
with `EngineResources::with_bunker_exit_policy` or `--bunker-exit-policy PATH`.
The policy supplies the transform and candidate offsets; no map coordinates are
embedded as a fallback. Missing policy yields `bunker_exit_policy_required`.
See `gameplay/src/vehicle_exit.rs` for the policy schema and validators.

## Horizon transport and lobby stack

Use the .NET 10 SDK:

```sh
dotnet build horizon/Horizon.Server.sln -c Release
dotnet run --project horizon/Tests.MagPlugin/Tests.MagPlugin.csproj -c Release
```

The synthetic check executable does not start Medius, DME, or a game client.
The solution build includes the server projects. See `horizon/README.md` for
runtime settings, certificate templates and development-key behavior.

## Source-only containers

```sh
docker build --target check -t mag-recomp-server-horizon-check horizon
docker build -t mag-recomp-server-horizon horizon
docker build -t mag-recomp-server-gameplay gameplay
```

These commands build local images and do not publish them. The Horizon check
target runs only synthetic tests. Both runtime images require external
configuration and resources before operation. They do not contain extracted
game data, authentication certificates, persistent accounts or packet captures.

## Publication checks

```sh
python3 -m unittest discover -s tests -v
python3 tools/check_public_tree.py
```

Run the guard after staging new files. In a Git checkout it checks tracked and
staged file paths against their working-tree contents. In an uninitialized
export it recursively checks source files. It rejects binary artifacts, private
runtime paths, private-key blobs, opaque encoded images and unmarked runtime
catalogs. It is a tripwire; passing it does not establish provenance by itself.
