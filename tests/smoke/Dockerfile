# syntax=docker/dockerfile:1
FROM rust:1.93-slim-bookworm

# System dependencies: build tools, git.
# No JRE needed — konvoy automatically downloads and bundles a JRE
# alongside the managed Kotlin/Native toolchain.
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential \
        curl \
        ca-certificates \
        git \
    && rm -rf /var/lib/apt/lists/*

# Copy project source and build konvoy.
WORKDIR /build
COPY . .
RUN cargo build --release

# Put konvoy on PATH.
ENV PATH="/build/target/release:${PATH}"

# Pre-install managed toolchain to avoid downloading ~500MB on every test run.
# This also serves as a smoke test of the install command itself.
RUN konvoy toolchain install 2.2.0

# Warm the konanc platform-libs cache (first compile downloads ~200MB).
RUN echo 'fun main() { println("warmup") }' > /tmp/w.kt \
    && mkdir -p /tmp/warmup \
    && printf '[package]\nname = "warmup"\n\n[toolchain]\nkotlin = "2.2.0"\n' > /tmp/warmup/konvoy.toml \
    && mkdir -p /tmp/warmup/src \
    && cp /tmp/w.kt /tmp/warmup/src/main.kt \
    && cd /tmp/warmup && konvoy build \
    && rm -rf /tmp/warmup /tmp/w.kt

# Warm the codegen path: pre-download the Fabrikt JAR, the kotlinx-serialization
# runtime klib, and the serialization compiler plugin, and compile a generated
# @Serializable model. Keeps the heavy test_codegen_build_lifecycle test off the
# cold-cache path and smoke-tests the codegen install path at image-build time.
# Uses BuildKit's heredoc RUN form (`RUN <<'SCRIPT'`) so the whole body is one
# shell script — nested heredocs (TOML/YAML) and newline-separated commands work
# without `\` continuations. `set -e` makes any failure abort the image build.
RUN <<'SCRIPT'
set -e
mkdir -p /tmp/cgwarm/src /tmp/cgwarm/specs
cat > /tmp/cgwarm/konvoy.toml <<'TOML'
[package]
name = "cgwarm"

[toolchain]
kotlin = "2.2.0"

[codegen.openapi]
version = "20.0.0"
spec = "specs/api.yaml"
base_package = "com.example.api"
spec_dirs = []

[dependencies]
kotlinx-serialization-core = { maven = "org.jetbrains.kotlinx:kotlinx-serialization-core", version = "1.7.3" }

[plugins]
serialization = { maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin", version = "{kotlin}" }
TOML
cat > /tmp/cgwarm/specs/api.yaml <<'YAML'
openapi: 3.0.3
info:
  title: Pet API
  version: 1.0.0
paths: {}
components:
  schemas:
    Pet:
      type: object
      required: [id, name]
      properties:
        id:
          type: integer
          format: int64
        name:
          type: string
YAML
printf 'fun main() { println("ok") }\n' > /tmp/cgwarm/src/main.kt
cd /tmp/cgwarm
konvoy build
rm -rf /tmp/cgwarm
SCRIPT

WORKDIR /workspace
