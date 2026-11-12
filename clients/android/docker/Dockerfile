# Build environment for the Android client: JDK 17 and the Android command-line tools.
# The SDK packages (platform, build tools, platform-tools) and Gradle's caches live in named
# volumes (see ../build.sh), so the image stays small and rebuilds keep their downloads.
# Pinned by digest, so a fresh build gets the same JDK; bump the digest (and build.sh rebuilds).
FROM eclipse-temurin:17-jdk-jammy@sha256:057a0daec60e87c96571bf5c9c7792065bcb19e804d584cd37d4144038a35717

ARG CMDLINE_TOOLS=11076708
ARG CMDLINE_TOOLS_SHA256=2d2d50857e4eb553af5a6dc3ad507a17adf43d115264b1afc116f95c92e5e258
RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends unzip curl ca-certificates git; \
    rm -rf /var/lib/apt/lists/*; \
    mkdir -p /opt/android-cmdline-tools; \
    curl -fsSL -o /tmp/tools.zip "https://dl.google.com/android/repository/commandlinetools-linux-${CMDLINE_TOOLS}_latest.zip"; \
    echo "${CMDLINE_TOOLS_SHA256}  /tmp/tools.zip" | sha256sum -c -; \
    unzip -q /tmp/tools.zip -d /opt/android-cmdline-tools; \
    mv /opt/android-cmdline-tools/cmdline-tools /opt/android-cmdline-tools/latest; \
    rm /tmp/tools.zip

# The SDK root is a volume; sdkmanager comes from the image.
ENV ANDROID_HOME=/opt/android-sdk \
    ANDROID_SDK_ROOT=/opt/android-sdk \
    GRADLE_USER_HOME=/gradle \
    PATH=/opt/android-cmdline-tools/latest/bin:/opt/android-sdk/platform-tools:$PATH
WORKDIR /src
