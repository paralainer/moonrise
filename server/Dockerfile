# Moonrise server: the binary next to ffmpeg. Multi-arch (linux/amd64, linux/arm64):
#
#   docker buildx build --platform linux/amd64,linux/arm64 -t ghcr.io/paralainer/moonrise-server:dev .
#   docker build -t moonrise-server .                      # just this machine's architecture
#   docker buildx build --platform linux/arm64 --load -t moonrise-server:arm64 .   # a Raspberry Pi's
#
# Nothing runs under emulation: the Rust build runs on the build machine's own architecture and
# cross-compiles for the target, and the arm64 image's ffmpeg is unpacked on the build machine
# (amd64 builds install jellyfin-ffmpeg with the target's own apt, which is the build machine's).

FROM --platform=$BUILDPLATFORM rust:1-bookworm AS build
ARG TARGETARCH
WORKDIR /src
# A cross linker for arm64 when building on amd64 (and the other way round).
RUN set -eux; \
    case "$TARGETARCH" in amd64) t=x86_64-unknown-linux-gnu; g=x86-64 ;; arm64) t=aarch64-unknown-linux-gnu; g=aarch64 ;; *) echo "unsupported $TARGETARCH"; exit 1 ;; esac; \
    echo "$t" > /rust-target; \
    if [ "$(uname -m | sed 's/_/-/')" != "$g" ]; then \
        apt-get update && apt-get install -y --no-install-recommends "gcc-$g-linux-gnu" "libc6-dev-$TARGETARCH-cross" && rm -rf /var/lib/apt/lists/*; \
    fi; \
    rustup target add "$t"
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
    AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
    CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc
COPY Cargo.toml Cargo.lock ./
COPY src src
COPY migrations migrations
COPY third_party third_party
# The project's built-in keys (MOONRISE_BUILTIN_TMDB_KEY, MOONRISE_BUILTIN_OPENSUBTITLES_KEY) come from an
# optional build secret `builtin_keys` (KEY=value lines), never a build argument: arguments stay
# in the image's history. Compiled into the binary; empty when the secret isn't given.
#   docker build --secret id=builtin_keys,src=keys.env .
# The registry cache is locked: the amd64 and arm64 builds run at once and unpacked the same
# crates into it together ("File exists").
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,id=moonrise-target-$TARGETARCH \
    --mount=type=secret,id=builtin_keys,required=false \
    set -a; if [ -f /run/secrets/builtin_keys ]; then . /run/secrets/builtin_keys; fi; set +a; \
    t=$(cat /rust-target); \
    cargo build --release --target "$t" && cp "target/$t/release/moonrise-server" /moonrise-server

# ---------------------------------------------------------------- arm64: Debian's ffmpeg
# jellyfin-ffmpeg's arm64 build has no V4L2 M2M encoder (the Raspberry Pi 4's hardware encoder),
# so arm64 images use Debian's ffmpeg (V4L2 M2M, libx264, zscale). Its packages are resolved
# against the arm64 base image's own package list and unpacked here, on the build machine; only
# ffmpeg, ffprobe and the libraries they load are kept.
FROM debian:trixie-slim AS base-arm64
FROM --platform=$BUILDPLATFORM debian:trixie-slim AS ffmpeg-arm64
COPY --from=base-arm64 /var/lib/dpkg/status /target/status
COPY --from=base-arm64 /usr/lib/aarch64-linux-gnu /base/usr/lib/aarch64-linux-gnu
RUN set -eux; \
    apt-get update; apt-get install -y --no-install-recommends binutils; \
    dpkg --add-architecture arm64; \
    mkdir -p /target/lists/partial /target/cache/archives/partial; \
    O="-o Dir::State::status=/target/status -o Dir::State::Lists=/target/lists -o Dir::Cache=/target/cache -o APT::Architecture=arm64 -o APT::Architectures::=arm64 -o APT::Install-Recommends=false"; \
    apt-get $O update; \
    apt-get $O install -y --download-only ffmpeg:arm64; \
    mkdir /full; for d in /target/cache/archives/*.deb; do dpkg-deb -x "$d" /full; done; \
    L=usr/lib/aarch64-linux-gnu; mkdir -p /rootfs/usr/bin "/rootfs/$L"; \
    cp /full/usr/bin/ffmpeg /full/usr/bin/ffprobe /rootfs/usr/bin/; \
    todo="/rootfs/usr/bin/ffmpeg /rootfs/usr/bin/ffprobe"; \
    while [ -n "$todo" ]; do \
      next=""; \
      for f in $todo; do \
        for n in $(readelf -d "$f" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'); do \
          [ -e "/rootfs/$L/$n" ] && continue; \
          [ -e "/base/$L/$n" ] && continue; \
          src=$(find /full/usr/lib /full/lib -name "$n" | head -1); \
          [ -n "$src" ] || { echo "missing $n (needed by $f)"; exit 1; }; \
          cp -L "$src" "/rootfs/$L/$n"; next="$next /rootfs/$L/$n"; \
        done; \
      done; \
      todo=$next; \
    done; \
    du -sh /rootfs

FROM debian:trixie-slim AS runtime-arm64
COPY --from=ffmpeg-arm64 /rootfs/ /

# ---------------------------------------------------------------- amd64: jellyfin-ffmpeg
FROM debian:trixie-slim AS runtime-amd64
# jellyfin-ffmpeg: Jellyfin's ffmpeg build, with NVENC/NVDEC and the scale_cuda, tonemap_cuda and
# overlay_cuda filters, Intel Quick Sync and VA-API (its own Intel and AMD drivers), libx264
# (Debian's ffmpeg lacks the CUDA filters). The NVIDIA driver libraries come from the NVIDIA
# container runtime (see docker-compose.yml). Pinned; bump deliberately.
ARG JELLYFIN_FFMPEG=jellyfin-ffmpeg8=8.1.3-1-trixie
RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates curl gnupg; \
    curl -fsSL https://repo.jellyfin.org/jellyfin_team.gpg.key | gpg --dearmor -o /usr/share/keyrings/jellyfin.gpg; \
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/jellyfin.gpg] https://repo.jellyfin.org/debian trixie main" > /etc/apt/sources.list.d/jellyfin.list; \
    apt-get update; \
    apt-get install -y --no-install-recommends "$JELLYFIN_FFMPEG"; \
    ln -s /usr/lib/jellyfin-ffmpeg/ffmpeg /usr/local/bin/ffmpeg; \
    ln -s /usr/lib/jellyfin-ffmpeg/ffprobe /usr/local/bin/ffprobe; \
    apt-get purge -y curl gnupg; apt-get autoremove -y; \
    rm -rf /var/lib/apt/lists/* /usr/share/doc /usr/share/man /usr/share/locale

# ---------------------------------------------------------------- the image
FROM runtime-$TARGETARCH
COPY --from=build /moonrise-server /usr/local/bin/moonrise-server
ENV MOONRISE_CACHE_DIR=/cache MOONRISE_DATA_DIR=/data
EXPOSE 8097
HEALTHCHECK --interval=60s --timeout=5s --start-period=30s CMD ["moonrise-server", "healthcheck"]
LABEL org.opencontainers.image.title="Moonrise server" \
      org.opencontainers.image.description="Media server for the Moonrise app (iPhone, iPad, Mac, Apple TV)" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later" \
      org.opencontainers.image.source="https://github.com/paralainer/moonrise"
ENTRYPOINT ["moonrise-server"]
