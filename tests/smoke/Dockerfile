# syntax=docker/dockerfile:1
FROM rust:1.93-slim-bookworm

# System dependencies: build tools, git, and a JRE for konanc.
# The kotlin-native-prebuilt tarball does NOT bundle a JRE; its run_konan
# script requires `java` (>= 8) on PATH.
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential \
        curl \
        ca-certificates \
        git \
        default-jre-headless \
    && rm -rf /var/lib/apt/lists/*

# Copy project source and build konvoy.
WORKDIR /build
COPY . .
RUN cargo build --release

# Put konvoy on PATH.
ENV PATH="/build/target/release:${PATH}"

# Pre-install managed toolchain to avoid downloading ~500MB on every test run.
# This also serves as a smoke test of the install command itself.
RUN konvoy toolchain install 2.1.0

# Warm the konanc platform-libs cache (first compile downloads ~200MB).
RUN echo 'fun main() { println("warmup") }' > /tmp/w.kt \
    && mkdir -p /tmp/warmup \
    && printf '[package]\nname = "warmup"\n\n[toolchain]\nkotlin = "2.1.0"\n' > /tmp/warmup/konvoy.toml \
    && mkdir -p /tmp/warmup/src \
    && cp /tmp/w.kt /tmp/warmup/src/main.kt \
    && cd /tmp/warmup && konvoy build \
    && rm -rf /tmp/warmup /tmp/w.kt

WORKDIR /workspace
