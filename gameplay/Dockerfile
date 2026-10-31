# Source-only build. Only independently authored synthetic defaults are embedded.
# Private runtime catalogs and collision artifacts must be mounted by the operator.
FROM rust:1.99.0-slim-bookworm@sha256:2c3a22f0a5533ea2dd5a16627bc841228151faa2d4de2644ac9987e4a2f1f2fa AS build
RUN apt-get update && apt-get install -y --no-install-recommends g++ ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /src/gameplay
COPY Cargo.toml Cargo.lock build.rs ./
COPY LICENSE ./
COPY src/ src/
COPY vendor/ vendor/
COPY data/synthetic-bot-objective-policy.json data/synthetic-wire-profiles.json data/synthetic-movement-metadata.json data/
RUN cargo build --locked --release --bins
RUN mkdir -p /out-licenses \
    && cp LICENSE /out-licenses/Project-MIT.txt \
    && cp vendor/recastnavigation/License.txt /out-licenses/RecastNavigation.txt \
    && find /usr/local/cargo/registry/src -type f \( -iname '*license*' -o -iname '*notice*' \) \
       -exec cp --parents -t /out-licenses/ '{}' +

FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587
RUN apt-get update && apt-get install -y --no-install-recommends libstdc++6 ca-certificates && rm -rf /var/lib/apt/lists/* && useradd --system --uid 10001 --create-home gameplay
COPY --from=build /src/gameplay/target/release/mag-gameplay /usr/local/bin/mag-gameplay
COPY --from=build /src/gameplay/target/release/mag-navmesh /usr/local/bin/mag-navmesh
COPY --from=build /out-licenses/ /usr/share/licenses/mag-recomp-server/
USER gameplay
WORKDIR /home/gameplay
ENTRYPOINT ["/usr/local/bin/mag-gameplay"]
