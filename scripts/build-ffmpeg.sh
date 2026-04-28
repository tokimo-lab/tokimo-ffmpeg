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
LOCAL_PATCHES_DIR="$ROOT_DIR/patches/local"
FFMPEG_GIT_URL="https://github.com/jellyfin/jellyfin-ffmpeg.git"
FFMPEG_REF="jellyfin"
JOBS="$(nproc 2>/dev/null || echo 4)"
ENABLE_NVIDIA=1
ENABLE_AMF=1
ENABLE_VULKAN=1
PATCHES_ONLY=0

log() { printf '[build] %s\n' "$*"; }
warn() { printf '[build] WARN: %s\n' "$*" >&2; }
die() { printf '[build] ERROR: %s\n' "$*" >&2; exit 1; }

# Prefer GNU patch (gpatch) when available — macOS ships BSD `patch`,
# which is stricter about fuzz / hunk reordering and rejects ~6 of the
# jellyfin debian patches that GNU patch handles fine. Linux/Windows
# already have GNU patch as `patch`, so this is a no-op there.
if command -v gpatch >/dev/null 2>&1; then
  PATCH_CMD="gpatch"
else
  PATCH_CMD="patch"
fi

# ─── Parse args ──────────────────────────────────────────────────
while (($# > 0)); do
  case "$1" in
    --src)       SRC_DIR="$2";     shift 2 ;;
    --build)     BUILD_DIR="$2";   shift 2 ;;
    --install)   INSTALL_DIR="$2"; shift 2 ;;
    --patches)   PATCHES_DIR="$2"; shift 2 ;;
    --local-patches) LOCAL_PATCHES_DIR="$2"; shift 2 ;;
    --ref)       FFMPEG_REF="$2";  shift 2 ;;
    --jobs)      JOBS="$2";        shift 2 ;;
    --no-nvidia) ENABLE_NVIDIA=0;  shift   ;;
    --no-amf)    ENABLE_AMF=0;     shift   ;;
    --no-vulkan) ENABLE_VULKAN=0;  shift   ;;
    --patches-only) PATCHES_ONLY=1; shift  ;;
    *) die "Unknown option: $1" ;;
  esac
done

# ─── Helpers ─────────────────────────────────────────────────────
pkg_exists() {
  command -v pkg-config >/dev/null 2>&1 && pkg-config --exists "$1"
}

append_if_pkg() {
  # $1: array name, $2: flag, $3: pkg
  # Uses eval-based indirect array append for bash 3.2 compatibility (macOS).
  if pkg_exists "$3"; then
    eval "$1+=(\"\$2\")"
  fi
}

append_if_any_pkg() {
  # $1: array name, $2: flag, $3+: pkg names
  local _arr="$1" _flag="$2"
  shift 2
  for pkg in "$@"; do
    if pkg_exists "$pkg"; then
      eval "$_arr+=(\"\$_flag\")"
      return
    fi
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
    if (cd "$SRC_DIR" && "$PATCH_CMD" --dry-run --reverse -p1 -s < "$patch_file" >/dev/null 2>&1); then
      ((skipped++)) || true
      continue
    fi

    if (cd "$SRC_DIR" && "$PATCH_CMD" --forward -p1 -s < "$patch_file" >/dev/null 2>&1); then
      ((applied++)) || true
    else
      warn "Patch conflict: $patch_name"
      ((failed++)) || true
    fi
  done < "$SERIES_FILE"
fi
log "Patches: $applied applied, $skipped already applied, $failed failed"

# ─── Step 3b: Apply Tokimo patches ──────────────────────────────
log "Step 3b/5: Applying tokimo patches from $LOCAL_PATCHES_DIR"
if [[ -d "$LOCAL_PATCHES_DIR" ]]; then
  shopt -s nullglob
  tokimo_patches=("$LOCAL_PATCHES_DIR"/*.patch)
  shopt -u nullglob
  if [[ ${#tokimo_patches[@]} -eq 0 ]]; then
    warn "No tokimo patches found in $LOCAL_PATCHES_DIR — refusing to continue (set --local-patches '' to skip explicitly)"
    if [[ "$LOCAL_PATCHES_DIR" != "" ]]; then
      die "Missing tokimo patches; aborting"
    fi
  fi
  for patch_file in "${tokimo_patches[@]}"; do
    patch_name="$(basename "$patch_file")"
    if (cd "$SRC_DIR" && "$PATCH_CMD" --dry-run --reverse -p1 -s < "$patch_file" >/dev/null 2>&1); then
      log "  Already applied: $patch_name"
      continue
    fi
    if (cd "$SRC_DIR" && "$PATCH_CMD" --forward -p1 -s < "$patch_file" >/dev/null 2>&1); then
      log "  Applied tokimo: $patch_name"
    else
      die "Tokimo patch FAILED: $patch_name"
    fi
  done
else
  warn "Tokimo patches dir not found: $LOCAL_PATCHES_DIR"
fi

if [[ "$PATCHES_ONLY" == "1" ]]; then
  log "Stop after patches (--patches-only); skipping third-party + configure + build."
  exit 0
fi

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
  log "  nv-codec-headers installed"
fi

# Always expose third-party prefix to pkg-config / compiler search paths
# (covers ffnvcodec.pc + Vulkan-Headers + future header-only deps).
if [[ -d "$TP_PREFIX" ]]; then
  export PKG_CONFIG_PATH="$TP_PREFIX/lib/pkgconfig:$TP_PREFIX/share/pkgconfig:${PKG_CONFIG_PATH:-}"
  export CPATH="$TP_PREFIX/include:${CPATH:-}"
fi

# Vulkan headers (need >= 1.3.277 for FFmpeg, Ubuntu 24.04 ships 1.3.275)
if [[ "$ENABLE_VULKAN" == "1" ]]; then
  VK_DIR="$THIRD_PARTY/Vulkan-Headers"
  if [[ -d "$VK_DIR/.git" ]]; then
    git -C "$VK_DIR" pull --rebase 2>/dev/null || true
  else
    git clone --depth 1 https://github.com/KhronosGroup/Vulkan-Headers.git "$VK_DIR"
  fi
  cmake -S "$VK_DIR" -B "$VK_DIR/build" -DCMAKE_INSTALL_PREFIX="$TP_PREFIX" -DCMAKE_BUILD_TYPE=Release >/dev/null 2>&1
  cmake --install "$VK_DIR/build" >/dev/null 2>&1
  log "  Vulkan-Headers installed ($(grep VK_HEADER_VERSION "$TP_PREFIX/include/vulkan/vulkan_core.h" | head -1 | awk '{print $3}'))"
else
  log "  Vulkan disabled (--no-vulkan); skipping Vulkan-Headers install"
fi

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

# On macOS, Homebrew installs to /opt/homebrew (Apple Silicon) or /usr/local
# (Intel). Several FFmpeg dependency probes — most notably libsoxr, which uses
# `require libsoxr soxr.h soxr_create -lsoxr` (raw header + lib check, not
# pkg-config) — fail if the compiler / linker do not search Homebrew's prefix.
# pkg-config-based probes work via PKG_CONFIG_PATH, but require/check_lib
# probes do not, so we have to inject the include / lib paths explicitly.
EXTRA_CFLAGS=""
EXTRA_LDFLAGS=""
if [[ "$(uname -s)" == "Darwin" ]] && command -v brew >/dev/null 2>&1; then
  BREW_PREFIX="$(brew --prefix 2>/dev/null || true)"
  if [[ -n "$BREW_PREFIX" && -d "$BREW_PREFIX/include" ]]; then
    EXTRA_CFLAGS="-I$BREW_PREFIX/include"
    EXTRA_LDFLAGS="-L$BREW_PREFIX/lib"
    log "  macOS: injecting Homebrew prefix into extra-cflags/ldflags ($BREW_PREFIX)"
  fi
fi

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
if [[ "$ENABLE_VULKAN" == "1" ]]; then
  append_if_pkg configure_flags "--enable-vulkan" vulkan
  append_if_pkg configure_flags "--enable-libplacebo" libplacebo
  append_if_any_pkg configure_flags "--enable-libshaderc" shaderc shaderc_combined
fi

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

if [[ -n "$EXTRA_CFLAGS" ]]; then
  configure_flags+=("--extra-cflags=$EXTRA_CFLAGS")
fi
if [[ -n "$EXTRA_LDFLAGS" ]]; then
  configure_flags+=("--extra-ldflags=$EXTRA_LDFLAGS")
fi

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
