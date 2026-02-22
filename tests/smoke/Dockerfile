# syntax=docker/dockerfile:1
FROM rust:1.85-slim-bookworm

# System dependencies: JDK (konanc needs it), linker, build tools.
RUN apt-get update && apt-get install -y --no-install-recommends \
        openjdk-17-jre-headless \
        build-essential \
        curl \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Install Kotlin/Native compiler.
ARG KOTLIN_VERSION=2.1.0
RUN curl -fsSL \
        "https://github.com/JetBrains/kotlin/releases/download/v${KOTLIN_VERSION}/kotlin-native-prebuilt-linux-x86_64-${KOTLIN_VERSION}.tar.gz" \
    | tar -xz -C /opt
ENV PATH="/opt/kotlin-native-prebuilt-linux-x86_64-${KOTLIN_VERSION}/bin:${PATH}"

# Warm the konanc platform-libs cache (first compile downloads ~200MB).
RUN echo 'fun main() { println("warmup") }' > /tmp/w.kt \
    && konanc /tmp/w.kt -o /tmp/w \
    && rm /tmp/w.kt /tmp/w.kexe

# Copy project source and build konvoy.
WORKDIR /build
COPY . .
RUN cargo build --release

# Put konvoy and test script on PATH / known location.
ENV PATH="/build/target/release:${PATH}"
WORKDIR /workspace
