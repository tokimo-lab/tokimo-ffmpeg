#!/usr/bin/env bash
# 安装 FFmpeg 编译依赖 (Ubuntu/Debian)
set -euo pipefail

log() { printf '[deps] %s\n' "$*"; }
die() { printf '[deps] ERROR: %s\n' "$*" >&2; exit 1; }

run_privileged() {
  if [[ "$(id -u)" -eq 0 ]]; then "$@"; return; fi
  if command -v sudo >/dev/null 2>&1; then sudo "$@"; return; fi
  die "Need root: $*"
}

[[ "$(uname -s)" == "Linux" ]] || die "This script only supports Linux"
command -v apt-get >/dev/null 2>&1 || die "Only apt-based distros are supported"

packages=(
  # Build tools
  git curl ca-certificates build-essential pkg-config nasm yasm clang
  autoconf automake libtool cmake meson ninja-build zlib1g-dev

  # GPU / Hardware acceleration
  libdrm-dev libva-dev libvulkan-dev libnuma-dev libvpl-dev libmfx-dev
  libvdpau-dev libplacebo-dev libshaderc-dev ocl-icd-opencl-dev

  # Subtitle & text rendering
  libass-dev libfreetype-dev libfribidi-dev libharfbuzz-dev
  libfontconfig1-dev libsdl2-dev

  # Audio devices
  libasound2-dev libpulse-dev libjack-jackd2-dev

  # Audio codecs
  libopus-dev libvorbis-dev libmp3lame-dev libsoxr-dev
  libfdk-aac-dev libtheora-dev libopenmpt-dev

  # Video codecs
  libbluray-dev libdav1d-dev libaom-dev libsvtav1-dev libsvtav1enc-dev
  libx264-dev libx265-dev libvpx-dev libwebp-dev

  # Image / misc
  libopenjp2-7-dev libjxl-dev
  libzimg-dev libchromaprint-dev
  libsrt-gnutls-dev libgnutls28-dev nettle-dev
  libzvbi-dev
)

log "Installing ${#packages[@]} packages..."
run_privileged apt-get update
run_privileged apt-get install -y "${packages[@]}"
log "Done."
