# Linux development environment with OCCT, matching CI.
#
#   docker build -t flatbello-dev tools/docker
#   docker run --rm -v "$PWD:/src" -v flatbello-target:/target -v flatbello-cargo:/root/.cargo/registry \
#       flatbello-dev cargo test --workspace --all-features
FROM ubuntu:26.04

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential pkg-config ca-certificates curl \
        libocct-foundation-dev libocct-modeling-data-dev \
        libocct-modeling-algorithms-dev libocct-data-exchange-dev \
        rustup \
    && rm -rf /var/lib/apt/lists/*
RUN rustup default stable && rustup component add rustfmt clippy

ENV CARGO_TARGET_DIR=/target
WORKDIR /src
