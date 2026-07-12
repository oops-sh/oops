# Linux test environment for oops.
#
# The OverlayFS snapshot backend only works on Linux, and the destructive
# integration tests must never run against the host filesystem, so the whole
# test suite runs inside this container (see `make test-linux`).
FROM rust:1.97-slim-trixie

RUN apt-get update && apt-get install -y --no-install-recommends \
    util-linux \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /oops
