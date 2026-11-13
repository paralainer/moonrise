# Builds the Android apps' Tailscale library (gomobile): the Android build image (JDK 17, the
# command-line tools) plus Go. The NDK goes into the cache volume (scripts/build-tailscale-android.sh).
ARG BASE=moonrise-android-build:1
FROM ${BASE}
ARG GO_VERSION=1.27.3
ARG GO_SHA256=65efbc80fdc1fa2cd2631cad5b481777a68e8725f34e89f50fc6ac714e379b40
RUN set -eux; \
    curl -fsSL -o /tmp/go.tgz "https://go.dev/dl/go${GO_VERSION}.linux-amd64.tar.gz"; \
    echo "${GO_SHA256}  /tmp/go.tgz" | sha256sum -c -; \
    tar -C /usr/local -xzf /tmp/go.tgz; \
    rm /tmp/go.tgz
ENV PATH=/usr/local/go/bin:$PATH GOTOOLCHAIN=local
