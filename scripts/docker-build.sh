#!/usr/bin/env bash
# Build patched jellyfin-ffmpeg in Docker and extract install/ to host.
# Usage: ./scripts/docker-build.sh [--no-cache]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

IMAGE_NAME="ffmpeg-builder"
NO_CACHE=""

log() { printf '\033[36m[docker-build]\033[0m %s\n' "$*"; }
die() { printf '\033[31m[docker-build] ERROR:\033[0m %s\n' "$*" >&2; exit 1; }

# ─── Parse args ──────────────────────────────────────────────────
for arg in "$@"; do
  case "$arg" in
    --no-cache) NO_CACHE="--no-cache" ;;
    -h|--help)
      echo "Usage: $0 [--no-cache]"
      echo "  --no-cache  Force full rebuild (ignore Docker layer and BuildKit caches)"
      exit 0
      ;;
    *) die "Unknown option: $arg (use --help)" ;;
  esac
done

# ─── Preflight checks ───────────────────────────────────────────
command -v docker >/dev/null 2>&1 || die "Docker not found. Install Docker first."
docker info >/dev/null 2>&1    || die "Docker daemon not running or insufficient permissions."

START_TIME=$(date +%s)

# ─── Build Docker image ─────────────────────────────────────────
cd "$ROOT_DIR"
log "Building Docker image '$IMAGE_NAME'..."
DOCKER_BUILDKIT=1 docker build \
  --progress=plain \
  ${NO_CACHE:+"$NO_CACHE"} \
  -t "$IMAGE_NAME" \
  .

# ─── Extract install/ from container ─────────────────────────────
log "Extracting install/ directory..."
rm -rf "$ROOT_DIR/install"
CONTAINER_ID=$(docker create "$IMAGE_NAME" /bin/true 2>/dev/null || docker create --entrypoint="" "$IMAGE_NAME" /bin/true 2>/dev/null)
trap "docker rm '$CONTAINER_ID' >/dev/null 2>&1 || true" EXIT
docker cp "$CONTAINER_ID:/install" "$ROOT_DIR/install"

# ─── Summary ─────────────────────────────────────────────────────
END_TIME=$(date +%s)
ELAPSED=$((END_TIME - START_TIME))

echo ""
log "════════════════════════════════════════════"
log "✅ Build complete! (${ELAPSED}s / $((ELAPSED / 60))m$((ELAPSED % 60))s)"
log "════════════════════════════════════════════"
echo ""
log "Output: $ROOT_DIR/install/"

if [[ -d "$ROOT_DIR/install/bin" ]]; then
  echo ""
  log "Binaries:"
  ls -lh "$ROOT_DIR/install/bin/" 2>/dev/null || true
fi

if [[ -d "$ROOT_DIR/install/lib" ]]; then
  SO_COUNT=$(find "$ROOT_DIR/install/lib" -name '*.so*' | wc -l)
  echo ""
  log "Shared libraries: $SO_COUNT files"
  ls -1 "$ROOT_DIR/install/lib/"*.so 2>/dev/null | head -10 || true
fi

if [[ -d "$ROOT_DIR/install/lib/pkgconfig" ]]; then
  PC_COUNT=$(ls -1 "$ROOT_DIR/install/lib/pkgconfig/"*.pc 2>/dev/null | wc -l)
  echo ""
  log "pkg-config files: $PC_COUNT"
fi

if [[ -d "$ROOT_DIR/install/include" ]]; then
  INC_COUNT=$(find "$ROOT_DIR/install/include" -name '*.h' | wc -l)
  echo ""
  log "Headers: $INC_COUNT files"
fi
