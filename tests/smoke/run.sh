#!/usr/bin/env bash
# Smoke test runner for Konvoy.
#
# Validates prerequisites, builds a Docker image with Rust + konanc + konvoy,
# and runs the test suite inside the container.
#
# Usage:
#   bash tests/smoke/run.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
IMAGE_NAME="konvoy-smoke"
MIN_DISK_MB=3000

# ---------------------------------------------------------------------------
# Colors
# ---------------------------------------------------------------------------
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

info()  { echo -e "${GREEN}==> $*${NC}"; }
warn()  { echo -e "${YELLOW}==> $*${NC}"; }
fail()  { echo -e "${RED}==> ERROR: $*${NC}" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Prereq checks
# ---------------------------------------------------------------------------
info "Checking prerequisites..."

# 1. Docker
if ! command -v docker &>/dev/null; then
    fail "Docker is not installed. Install Docker: https://docs.docker.com/get-docker/"
fi
if ! docker info &>/dev/null; then
    fail "Docker daemon is not running. Start Docker and try again."
fi
echo "  [ok] Docker is installed and running"

# 2. BuildKit (docker buildx)
if ! docker buildx version &>/dev/null; then
    fail "Docker BuildKit (buildx) is not available. Update Docker or install the buildx plugin."
fi
echo "  [ok] Docker BuildKit is available"

# 3. Disk space
available_mb=$(df -m "$PROJECT_ROOT" | awk 'NR==2 {print $4}')
if [ "$available_mb" -lt "$MIN_DISK_MB" ]; then
    fail "Insufficient disk space: ${available_mb}MB available, ${MIN_DISK_MB}MB required."
fi
echo "  [ok] Disk space: ${available_mb}MB available (${MIN_DISK_MB}MB required)"

echo ""

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
info "Building smoke test image (this may take a few minutes on first run)..."

docker buildx build \
    --load \
    -t "$IMAGE_NAME" \
    -f "$SCRIPT_DIR/Dockerfile" \
    "$PROJECT_ROOT"

echo ""

# ---------------------------------------------------------------------------
# Run tests
# ---------------------------------------------------------------------------
info "Running smoke tests..."
echo ""

docker run --rm \
    "$IMAGE_NAME" \
    bash /build/tests/smoke/tests.sh

echo ""
info "All smoke tests passed."
