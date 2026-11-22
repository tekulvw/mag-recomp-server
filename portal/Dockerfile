# Source-only build. The image holds no credentials, keys or account data; the operator
# supplies the database, the Google client secret and the ticket signing key at run time.
FROM rust:1.99.0-slim-bookworm@sha256:2c3a22f0a5533ea2dd5a16627bc841228151faa2d4de2644ac9987e4a2f1f2fa AS build
RUN apt-get update && apt-get install -y --no-install-recommends gcc g++ make ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /src/portal
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src/ src/
COPY migrations/ migrations/
COPY templates/ templates/
COPY static/ static/
COPY tests/ tests/
RUN cargo build --locked --release --bin mag-portal

# Synthetic checks against a throwaway PostgreSQL started inside this build step.
# This target never contacts Google or a game server.
FROM build AS check
RUN apt-get update && apt-get install -y --no-install-recommends postgresql && rm -rf /var/lib/apt/lists/*
# rust-toolchain.toml makes rustup install rustfmt and clippy on first use.
RUN cargo fmt --all -- --check \
    && cargo clippy --locked --all-targets -- -D warnings
RUN set -eu; \
    pg="$(ls -d /usr/lib/postgresql/*/bin | head -n1)"; \
    mkdir -p /tmp/pg && chown postgres /tmp/pg; \
    su postgres -c "$pg/initdb -D /tmp/pg/data -U portal --auth=trust >/dev/null"; \
    su postgres -c "$pg/pg_ctl -D /tmp/pg/data -o '-k /tmp/pg -c listen_addresses=127.0.0.1' -w start >/dev/null"; \
    DATABASE_URL=postgres://portal@127.0.0.1:5432/postgres cargo test --locked; \
    su postgres -c "$pg/pg_ctl -D /tmp/pg/data -m fast stop >/dev/null"

FROM build AS licenses
RUN mkdir -p /out-licenses \
    && find /usr/local/cargo/registry/src -type f \( -iname '*license*' -o -iname '*notice*' \) \
       -exec cp --parents -t /out-licenses/ '{}' +

FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home portal
COPY --from=build /src/portal/target/release/mag-portal /usr/local/bin/mag-portal
COPY --from=licenses /out-licenses/ /usr/share/licenses/mag-recomp-server/
COPY LICENSE /usr/share/licenses/mag-recomp-server/Project-MIT.txt
USER portal
WORKDIR /home/portal
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/mag-portal"]
CMD ["serve"]
