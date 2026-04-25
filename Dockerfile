# syntax=docker/dockerfile:1
# Multi-stage Docker build for patched jellyfin-ffmpeg
# Requires BuildKit: DOCKER_BUILDKIT=1 docker build ...

# ── Stage 1: Builder ─────────────────────────────────────────────
FROM nvidia/cuda:12.8.1-devel-ubuntu24.04 AS builder

ENV DEBIAN_FRONTEND=noninteractive

# Install build dependencies (mirrors scripts/install-deps.sh) + ccache
RUN apt-get update && apt-get install -y --no-install-recommends \
    # Build tools
    git curl ca-certificates build-essential pkg-config nasm yasm clang \
    autoconf automake libtool cmake meson ninja-build zlib1g-dev \
    # GPU / Hardware acceleration
    libdrm-dev libva-dev libvulkan-dev libnuma-dev libvpl-dev libmfx-dev \
    libvdpau-dev libplacebo-dev libshaderc-dev ocl-icd-opencl-dev \
    # Subtitle & text rendering
    libass-dev libfreetype-dev libfribidi-dev libharfbuzz-dev \
    libfontconfig1-dev libsdl2-dev \
    # Audio devices
    libasound2-dev libpulse-dev libjack-jackd2-dev \
    # Audio codecs
    libopus-dev libvorbis-dev libmp3lame-dev libsoxr-dev \
    libfdk-aac-dev libtheora-dev libopenmpt-dev \
    # Video codecs
    libbluray-dev libdav1d-dev libaom-dev libsvtav1-dev libsvtav1enc-dev \
    libx264-dev libx265-dev libvpx-dev libwebp-dev \
    # Image / misc
    libopenjp2-7-dev libjxl-dev \
    libzimg-dev libchromaprint-dev \
    libsrt-gnutls-dev libgnutls28-dev nettle-dev \
    libzvbi-dev \
    # Docker-specific extras
    ccache \
    && rm -rf /var/lib/apt/lists/*

# Use ccache transparently via PATH (Ubuntu ccache symlinks)
ENV CCACHE_DIR=/root/.cache/ccache \
    PATH="/usr/lib/ccache:${PATH}"

WORKDIR /workspace

# Copy build scripts and patches into container
COPY scripts/ scripts/
COPY patches/ patches/

# Build FFmpeg with BuildKit cache mounts:
#  - ccache: avoid recompiling unchanged source files
#  - ffmpeg-src: avoid re-cloning the large git repo (~1GB)
#  - third-party: avoid re-cloning nv-codec-headers and AMF
RUN --mount=type=cache,target=/root/.cache/ccache \
    --mount=type=cache,target=/workspace/ffmpeg-src \
    --mount=type=cache,target=/workspace/third-party \
    ./scripts/build-ffmpeg.sh

# ── Stage 2: Output (minimal image with just the install tree) ───
FROM scratch AS output
COPY --from=builder /workspace/install /install
