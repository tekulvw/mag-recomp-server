# Runner image for this repository's self-hosted Actions pool (runs-on: mag-recomp-server).
# Built and published by .github/workflows/runner-image.yml as
# ghcr.io/tekulvw/mag-recomp-server-runner; the runner scale set pins one digest of it.
#
# It extends the shared ARC runner image, which already has actions-runner, the
# apt build tools that .github/actions/self-hosted-linux installs, and Rust 1.99.0
# with rustfmt and clippy. Building on it lets nodes reuse those layers from the
# other pools. This layer adds only what this repository's jobs would otherwise
# install on every run: PostgreSQL for the portal tests (about 100 s per job).
#
# Bump BASE_IMAGE when the shared runner image moves to a new actions/runner
# release: GitHub refuses runner versions some time after a newer release.
ARG BASE_IMAGE=ghcr.io/tekulvw/arc-runner-mag:2.338.2-5@sha256:d355cf339d56f8c603fff38f0ba0ddf4e38e9b60f58118463b0943510ef9482c
FROM ${BASE_IMAGE}

USER root
# The portal job initializes its own throwaway cluster from /usr/lib/postgresql/*/bin.
RUN apt-get update -q \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-install-recommends postgresql \
    && ls -d /usr/lib/postgresql/*/bin/initdb
# The apt lists stay in the image, as in the base image: every job still runs
# `apt-get update`, which then fetches only what changed since this build.

USER runner
