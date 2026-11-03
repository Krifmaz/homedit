# syntax=docker/dockerfile:1.7
#
# homedit container image: one static binary with the web UI embedded.
#
# Build for the local machine:
#   docker build -t homedit .
# Build for amd64 and arm64:
#   docker buildx build --platform linux/amd64,linux/arm64 -t homedit .
#
# The Rust stage always runs on the build machine and cross-compiles to the
# target with cargo-zigbuild, so arm64 images build without QEMU emulation.
# The final image is `scratch`: no shell, no package manager, only the
# binary, CA certificates and an empty /data volume owned by an unprivileged
# user.

ARG RUST_VERSION=1.99.1
ARG NODE_VERSION=24
ARG ZIGBUILD_VERSION=0.23.4

# ---------------------------------------------------------------------------
# 1. Web UI (React + Vite). Platform independent, so it runs natively.
# ---------------------------------------------------------------------------
FROM --platform=$BUILDPLATFORM node:${NODE_VERSION}-alpine AS web
WORKDIR /web
# Install dependencies first so the layer is cached until the lockfile
# changes. The bind mount means the build works before web/ has a
# package.json (the placeholder web/dist is used as-is then).
RUN --mount=type=bind,source=web,target=/src \
    --mount=type=cache,target=/root/.npm \
    if [ -f /src/package.json ]; then \
        cp /src/package.json /src/package-lock.json* ./ && \
        if [ -f package-lock.json ]; then npm ci --no-audit --no-fund; \
        else npm install --no-audit --no-fund; fi; \
    fi
COPY web/ ./
RUN if [ -f package.json ]; then npm run build; fi && \
    mkdir -p dist && \
    test -n "$(ls -A dist)" || touch dist/.gitkeep

# ---------------------------------------------------------------------------
# 2. Rust server, cross-compiled to a static musl binary.
# ---------------------------------------------------------------------------
# cargo-zigbuild uses Zig as the C cross compiler and linker, so the C parts
# of the dependency tree (bundled SQLite, the TLS crypto backend) build for
# amd64 and arm64 from one image, with no distro packages to install.
FROM --platform=$BUILDPLATFORM messense/cargo-zigbuild:${ZIGBUILD_VERSION} AS build
ARG RUST_VERSION
ARG TARGETARCH
RUN case "$TARGETARCH" in \
        amd64) echo x86_64-unknown-linux-musl ;; \
        arm64) echo aarch64-unknown-linux-musl ;; \
        *) echo "unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac > /rust-target && \
    rustup toolchain install "$RUST_VERSION" --profile minimal --target "$(cat /rust-target)" && \
    rustup default "$RUST_VERSION"
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY --from=web /web/dist ./web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,id=homedit-target-${TARGETARCH},sharing=locked \
    cargo zigbuild --release --locked -p homedit --bin homedit --target "$(cat /rust-target)" && \
    cp "target/$(cat /rust-target)/release/homedit" /homedit
# The data volume, owned by the runtime user (scratch has no mkdir), the CA
# bundle for outbound HTTPS, and passwd/group entries for the runtime user.
RUN mkdir -p /rootfs/data /rootfs/etc/ssl/certs && \
    cp /etc/ssl/certs/ca-certificates.crt /rootfs/etc/ssl/certs/ && \
    printf 'homedit:x:65532:65532:homedit:/data:/sbin/nologin\n' > /rootfs/etc/passwd && \
    printf 'homedit:x:65532:\n' > /rootfs/etc/group

# ---------------------------------------------------------------------------
# 3. Runtime image.
# ---------------------------------------------------------------------------
FROM scratch
ARG VERSION=dev
ARG REVISION=unknown
LABEL org.opencontainers.image.title="homedit" \
      org.opencontainers.image.description="Self-hosted homelab dashboard with a built-in editor and service discovery" \
      org.opencontainers.image.source="https://github.com/Krifmaz/homedit" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}"
COPY --from=build /rootfs/etc /etc
COPY --from=build --chown=65532:65532 /rootfs/data /data
COPY --from=build /homedit /homedit
ENV HOMEDIT_LISTEN=0.0.0.0:3000 \
    HOMEDIT_DATA_DIR=/data \
    SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt \
    RUST_LOG=info
USER 65532:65532
VOLUME ["/data"]
EXPOSE 3000
# `homedit healthcheck` sends GET /healthz to HOMEDIT_LISTEN and exits 0 on
# a 200 answer; the image has no curl or shell.
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["/homedit", "healthcheck"]
ENTRYPOINT ["/homedit"]
