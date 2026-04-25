#!/usr/bin/env bash
# 一键编译 jellyfin-ffmpeg (patched)
# Usage: ./scripts/build-ffmpeg.sh [options]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Defaults
SRC_DIR="$ROOT_DIR/ffmpeg-src"
BUILD_DIR="$ROOT_DIR/build"
INSTALL_DIR="$ROOT_DIR/install"
PATCHES_DIR="$ROOT_DIR/patches"
FFMPEG_GIT_URL="https://github.com/jellyfin/jellyfin-ffmpeg.git"
FFMPEG_REF="jellyfin"
JOBS="$(nproc 2>/dev/null || echo 4)"
ENABLE_NVIDIA=1
ENABLE_AMF=1

log() { printf '[build] %s\n' "$*"; }
warn() { printf '[build] WARN: %s\n' "$*" >&2; }
die() { printf '[build] ERROR: %s\n' "$*" >&2; exit 1; }

# ─── Parse args ──────────────────────────────────────────────────
while (($# > 0)); do
  case "$1" in
    --src)       SRC_DIR="$2";     shift 2 ;;
    --build)     BUILD_DIR="$2";   shift 2 ;;
    --install)   INSTALL_DIR="$2"; shift 2 ;;
    --patches)   PATCHES_DIR="$2"; shift 2 ;;
    --ref)       FFMPEG_REF="$2";  shift 2 ;;
    --jobs)      JOBS="$2";        shift 2 ;;
    --no-nvidia) ENABLE_NVIDIA=0;  shift   ;;
    --no-amf)    ENABLE_AMF=0;     shift   ;;
    *) die "Unknown option: $1" ;;
  esac
done

# ─── Helpers ─────────────────────────────────────────────────────
pkg_exists() {
  command -v pkg-config >/dev/null 2>&1 && pkg-config --exists "$1"
}

append_if_pkg() {
  local -n arr="$1"
  local flag="$2" pkg="$3"
  if pkg_exists "$pkg"; then arr+=("$flag"); fi
}

append_if_any_pkg() {
  local -n arr="$1"
  local flag="$2"
  shift 2
  for pkg in "$@"; do
    if pkg_exists "$pkg"; then arr+=("$flag"); return; fi
  done
}

# ─── Step 1: Clone / Update Source ───────────────────────────────
log "Step 1/5: Syncing jellyfin-ffmpeg source (ref: $FFMPEG_REF)"
if [[ -d "$SRC_DIR/.git" ]]; then
  git -C "$SRC_DIR" fetch --tags --prune origin
else
  # Clear contents without removing the directory (supports Docker cache mounts)
  find "$SRC_DIR" -mindepth 1 -delete 2>/dev/null || rm -rf "$SRC_DIR"/* "$SRC_DIR"/.[!.]* 2>/dev/null || true
  git clone "$FFMPEG_GIT_URL" "$SRC_DIR"
fi

if git -C "$SRC_DIR" rev-parse --verify "refs/tags/$FFMPEG_REF" >/dev/null 2>&1; then
  git -C "$SRC_DIR" checkout --force "refs/tags/$FFMPEG_REF"
elif git -C "$SRC_DIR" rev-parse --verify "refs/remotes/origin/$FFMPEG_REF" >/dev/null 2>&1; then
  git -C "$SRC_DIR" checkout --force -B "$FFMPEG_REF" "origin/$FFMPEG_REF"
else
  git -C "$SRC_DIR" checkout --force "$FFMPEG_REF"
fi

# ─── Step 2: Extract Patches for Viewing ────────────────────────
log "Step 2/5: Extracting patches to $PATCHES_DIR/"
"$SCRIPT_DIR/extract-patches.sh" "$SRC_DIR" "$PATCHES_DIR"

# ─── Step 3: Apply Patches ──────────────────────────────────────
log "Step 3/5: Applying debian patches"
SERIES_FILE="$SRC_DIR/debian/patches/series"
applied=0 skipped=0 failed=0

if [[ -f "$SERIES_FILE" ]]; then
  while IFS= read -r patch_name || [[ -n "$patch_name" ]]; do
    [[ -z "$patch_name" || "$patch_name" == \#* ]] && continue
    patch_file="$SRC_DIR/debian/patches/$patch_name"
    [[ -f "$patch_file" ]] || continue

    # Already applied?
    if (cd "$SRC_DIR" && patch --dry-run --reverse -p1 -s < "$patch_file" >/dev/null 2>&1); then
      ((skipped++)) || true
      continue
    fi

    if (cd "$SRC_DIR" && patch --forward -p1 -s < "$patch_file" >/dev/null 2>&1); then
      ((applied++)) || true
    else
      warn "Patch conflict: $patch_name"
      ((failed++)) || true
    fi
  done < "$SERIES_FILE"
fi
log "Patches: $applied applied, $skipped already applied, $failed failed"

# ─── Step 4: Setup Third-Party Headers ──────────────────────────
log "Step 4/5: Setting up third-party headers"
THIRD_PARTY="$ROOT_DIR/third-party"
TP_PREFIX="$THIRD_PARTY/prefix"
mkdir -p "$THIRD_PARTY" "$TP_PREFIX"

# nv-codec-headers
if [[ "$ENABLE_NVIDIA" == "1" ]]; then
  NV_DIR="$THIRD_PARTY/nv-codec-headers"
  if [[ -d "$NV_DIR/.git" ]]; then
    git -C "$NV_DIR" pull --rebase 2>/dev/null || true
  else
    git clone https://github.com/FFmpeg/nv-codec-headers.git "$NV_DIR"
  fi
  make -C "$NV_DIR" PREFIX="$TP_PREFIX" install
  export PKG_CONFIG_PATH="$TP_PREFIX/lib/pkgconfig:${PKG_CONFIG_PATH:-}"
  log "  nv-codec-headers installed"
fi

# Vulkan headers (need >= 1.3.277 for FFmpeg, Ubuntu 24.04 ships 1.3.275)
VK_DIR="$THIRD_PARTY/Vulkan-Headers"
if [[ -d "$VK_DIR/.git" ]]; then
  git -C "$VK_DIR" pull --rebase 2>/dev/null || true
else
  git clone --depth 1 https://github.com/KhronosGroup/Vulkan-Headers.git "$VK_DIR"
fi
cmake -S "$VK_DIR" -B "$VK_DIR/build" -DCMAKE_INSTALL_PREFIX="$TP_PREFIX" -DCMAKE_BUILD_TYPE=Release >/dev/null 2>&1
cmake --install "$VK_DIR/build" >/dev/null 2>&1
log "  Vulkan-Headers installed ($(grep VK_HEADER_VERSION "$TP_PREFIX/include/vulkan/vulkan_core.h" | head -1 | awk '{print $3}'))"

# AMF headers
if [[ "$ENABLE_AMF" == "1" ]]; then
  AMF_DIR="$THIRD_PARTY/AMF"
  AMF_INC="$THIRD_PARTY/include/AMF"
  if [[ -d "$AMF_DIR/.git" ]]; then
    git -C "$AMF_DIR" pull --rebase 2>/dev/null || true
  else
    git clone https://github.com/GPUOpen-LibrariesAndSDKs/AMF.git "$AMF_DIR"
  fi
  rm -rf "$AMF_INC"
  mkdir -p "$AMF_INC"
  cp -R "$AMF_DIR/amf/public/include/." "$AMF_INC/"
  export CPATH="$THIRD_PARTY/include:${CPATH:-}"
  log "  AMF headers installed"
fi

# ─── Step 5: Configure & Build ──────────────────────────────────
log "Step 5/5: Configuring and building FFmpeg (jobs=$JOBS)"

HOST_ARCH="$(uname -m)"
case "$HOST_ARCH" in
  x86_64|amd64) HOST_ARCH="x86_64" ;;
  arm64|aarch64) HOST_ARCH="aarch64" ;;
esac

rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR" "$INSTALL_DIR"

configure_flags=(
  "--prefix=$INSTALL_DIR"
  "--enable-gpl"
  "--enable-version3"
  "--enable-nonfree"
  "--enable-rpath"
  "--enable-shared"
  "--disable-static"
  "--enable-pic"
  "--disable-doc"
  "--disable-debug"
  "--arch=$HOST_ARCH"
  "--enable-lto=auto"
  "--extra-version=Jellyfin"
  "--disable-ffplay"
  "--disable-libxcb"
  "--disable-xlib"
)

# Video codecs
append_if_pkg configure_flags "--enable-libx264" x264
append_if_pkg configure_flags "--enable-libx265" x265
append_if_pkg configure_flags "--enable-libdav1d" dav1d
append_if_any_pkg configure_flags "--enable-libsvtav1" SvtAv1Enc svtav1
append_if_pkg configure_flags "--enable-libvpx" vpx
append_if_pkg configure_flags "--enable-libaom" aom

# Audio codecs
append_if_pkg configure_flags "--enable-libopus" opus
append_if_any_pkg configure_flags "--enable-libvorbis" vorbis vorbisenc
append_if_any_pkg configure_flags "--enable-libmp3lame" mp3lame lame
append_if_pkg configure_flags "--enable-libfdk-aac" fdk-aac
append_if_pkg configure_flags "--enable-libtheora" theoraenc
append_if_pkg configure_flags "--enable-libopenmpt" libopenmpt
append_if_pkg configure_flags "--enable-libsoxr" soxr

# Subtitle & text
append_if_pkg configure_flags "--enable-libdrm" libdrm
append_if_pkg configure_flags "--enable-libass" libass
append_if_pkg configure_flags "--enable-libfontconfig" fontconfig
append_if_pkg configure_flags "--enable-libfreetype" freetype2
append_if_pkg configure_flags "--enable-libfribidi" fribidi
append_if_pkg configure_flags "--enable-libharfbuzz" harfbuzz

# Container / image / misc
append_if_pkg configure_flags "--enable-libbluray" libbluray
append_if_pkg configure_flags "--enable-libwebp" libwebp
append_if_pkg configure_flags "--enable-libzimg" zimg
append_if_any_pkg configure_flags "--enable-chromaprint" libchromaprint chromaprint
append_if_pkg configure_flags "--enable-libsrt" srt
append_if_pkg configure_flags "--enable-libzvbi" zvbi-0.2
append_if_pkg configure_flags "--enable-libopenjpeg" libopenjp2
append_if_pkg configure_flags "--enable-libjxl" libjxl

# Audio devices
append_if_pkg configure_flags "--enable-libjack" jack
append_if_pkg configure_flags "--enable-libpulse" libpulse

# GPU: Vulkan + libplacebo
append_if_pkg configure_flags "--enable-vulkan" vulkan
append_if_pkg configure_flags "--enable-libplacebo" libplacebo
append_if_any_pkg configure_flags "--enable-libshaderc" shaderc shaderc_combined

# GPU: OpenCL
append_if_pkg configure_flags "--enable-opencl" OpenCL

# GPU: VAAPI
append_if_pkg configure_flags "--enable-vaapi" libva

# GPU: NVIDIA
if pkg_exists ffnvcodec; then
  configure_flags+=(
    "--enable-ffnvcodec"
    "--enable-cuda"
    "--enable-cuda-llvm"
    "--enable-cuvid"
    "--enable-nvdec"
    "--enable-nvenc"
  )
fi

# GPU: AMD AMF
if [[ "$ENABLE_AMF" == "1" ]]; then
  configure_flags+=("--enable-amf")
fi

# Intel QSV
if pkg_exists vpl; then
  configure_flags+=("--enable-libvpl")
elif pkg_exists libmfx; then
  configure_flags+=("--enable-libmfx")
fi

cd "$BUILD_DIR"

log "Running configure with ${#configure_flags[@]} flags..."
if ! "$SRC_DIR/configure" "${configure_flags[@]}" > /tmp/configure.log 2>&1; then
  log "Configure FAILED. Last 40 lines:"
  tail -40 /tmp/configure.log
  die "configure failed"
fi
tail -5 /tmp/configure.log

log "Building... (this will take a while)"
if ! make -j"$JOBS" > /tmp/make.log 2>&1; then
  log "Build FAILED. Last 40 lines:"
  tail -40 /tmp/make.log
  die "make failed"
fi
tail -3 /tmp/make.log

log "Installing to $INSTALL_DIR/"
make install 2>&1 | tail -3

# ─── Summary ────────────────────────────────────────────────────
FFBIN="$INSTALL_DIR/bin/ffmpeg"
if [[ -x "$FFBIN" ]]; then
  log "✅ Build successful!"
  echo ""
  export LD_LIBRARY_PATH="$INSTALL_DIR/lib:${LD_LIBRARY_PATH:-}"
  "$FFBIN" -hide_banner -version | head -3
  echo ""
  log "Run 'make info' for full capability details"
else
  die "Build completed but ffmpeg binary not found at $FFBIN"
fi
