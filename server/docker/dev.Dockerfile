# The dev image for the server's tests and lint: the pinned Rust toolchain plus clippy and rustfmt.
# Not the production image (server/Dockerfile). scripts/rust-dev.sh builds it on first use, as
# moonrise-rust-dev:<version>, with RUST_VERSION read from server/rust-toolchain.toml:
#
#   docker build -f server/docker/dev.Dockerfile --build-arg RUST_VERSION=1.99.1 \
#     -t moonrise-rust-dev:1.99.1 server/docker
ARG RUST_VERSION
FROM rust:${RUST_VERSION}-bookworm
RUN rustup component add clippy rustfmt
