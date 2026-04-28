#!/usr/bin/env bash
# Cross-compile patched jellyfin-ffmpeg for Windows (x86_64-w64-mingw32)
# using BtbN/FFmpeg-Builds' public docker image which already ships:
#   - mingw-w64 GCC 15 toolchain (crosstool-NG, /opt/ct-ng)
#   - all GPL codec deps prebuilt at /opt/ffbuild/{lib,include}
#     (x264/x265/dav1d/svt-av1/aom/vpx/opus/lame/vorbis/theora/openmpt/
#      soxr/ass/freetype/fribidi/harfbuzz/fontconfig/bluray/webp/zimg/
#      srt/openjpeg/libjxl/chromaprint, plus extras like ffnvcodec/AMF/
#      vulkan/libvpl)
#
# Why this image?
#   `win64-nonfree-shared` is NOT published on ghcr.io because fdk-aac is
#   non-redistributable. We use `win64-gpl-shared` (which has every codec
#   we need EXCEPT fdk-aac) and build fdk-aac from source ourselves
#   inside the same container, then run `--enable-libfdk-aac
#   --enable-nonfree` against the patched jellyfin-ffmpeg tree that
#   `build-ffmpeg.sh --patches-only` left in `ffmpeg-src/`.
#
# Outputs:
#   install-windows/{bin,lib,include}    — dlls, .dll.a import libs, headers
#
# Inputs:
#   ffmpeg-src/                          — patched FFmpeg tree
#                                          (run scripts/build-ffmpeg.sh
#                                           --patches-only first)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

SRC_DIR="${SRC_DIR:-$ROOT_DIR/ffmpeg-src}"
INSTALL_DIR="${INSTALL_DIR:-$ROOT_DIR/install-windows}"
IMAGE="${BTBN_IMAGE:-ghcr.io/btbn/ffmpeg-builds/win64-gpl-shared:latest}"
FDK_AAC_REF="${FDK_AAC_REF:-d8e6b1a3aa606c450241632b64b703f21ea31ce3}"

if [[ ! -d "$SRC_DIR" ]]; then
  echo "[cross-win] ERROR: $SRC_DIR not found. Run scripts/build-ffmpeg.sh --patches-only first." >&2
  exit 1
fi

mkdir -p "$INSTALL_DIR"
rm -rf "$ROOT_DIR/build-windows"
mkdir -p "$ROOT_DIR/build-windows"

echo "[cross-win] Pulling $IMAGE"
docker pull "$IMAGE"

# Run uid:gid of host user when not rootless, so output files are owned by host user.
UIDARGS=()
if ! docker info -f '{{println .SecurityOptions}}' 2>/dev/null | grep -q rootless; then
  UIDARGS=( -u "$(id -u):$(id -g)" )
fi

# Run the build inside the container. The container already provides all
# tooling and most codec deps; we only fetch + build fdk-aac, then run
# our own configure with the jellyfin-ffmpeg flag set.
docker run --rm "${UIDARGS[@]}" \
  -v "$SRC_DIR":/work/ffmpeg-src \
  -v "$ROOT_DIR/build-windows":/work/build \
  -v "$INSTALL_DIR":/work/install \
  -e FDK_AAC_REF="$FDK_AAC_REF" \
  -e FDK_PREFIX=/work/build/fdk-aac-prefix \
  -w /work \
  "$IMAGE" \
  bash -eo pipefail -c '
    set -x
    : "${FFBUILD_PREFIX:?image must define FFBUILD_PREFIX}"
    : "${FFBUILD_TOOLCHAIN:?image must define FFBUILD_TOOLCHAIN}"
    : "${FFBUILD_TARGET_FLAGS:?image must define FFBUILD_TARGET_FLAGS}"
    : "${CC:?image must define CC}"

    : "${FDK_PREFIX:?must be set}"
    nproc_count=$(nproc)

    # ── 1. Build fdk-aac into a user-writable prefix ───────────────────
    # mstorsjo/fdk-aac is plain autotools, ~30s wall clock under -j$(nproc).
    # We pin to the same commit BtbN uses in scripts.d/50-fdk-aac.sh so
    # the binding stays reproducible across runs.
    #
    # Install prefix is FDK_PREFIX=/work/build/fdk-aac-prefix (host-mounted)
    # — we cannot write to $FFBUILD_PREFIX (=/opt/ffbuild, root-owned)
    # when the container runs as the host UID via -u $(id -u):$(id -g).
    if [[ ! -f "$FDK_PREFIX/lib/libfdk-aac.a" ]]; then
      mkdir -p /work/build/fdk-aac
      cd /work/build/fdk-aac
      git clone --filter=blob:none https://github.com/mstorsjo/fdk-aac.git src
      cd src
      git checkout "$FDK_AAC_REF"
      ./autogen.sh
      ./configure \
        --prefix="$FDK_PREFIX" \
        --host="$FFBUILD_TOOLCHAIN" \
        --disable-shared \
        --enable-static \
        --with-pic \
        --disable-example
      make -j"$nproc_count"
      make install
    fi
    # Make fdk-aac visible to FFmpegs pkg-config / configure probes,
    # alongside the image-provided $FFBUILD_PREFIX deps.
    export PKG_CONFIG_PATH="$FDK_PREFIX/lib/pkgconfig:${PKG_CONFIG_PATH:-}"

    # ── 2. Configure & build patched FFmpeg (jellyfin tree) ────────────
    cd /work/build
    rm -rf ff && mkdir ff && cd ff

    # GPU stack on Windows. The BtbN win64-gpl-shared image already ships
    # all required headers / import libs at $FFBUILD_PREFIX:
    #   - vulkan + libplacebo + libshaderc  (Vulkan compute / shader pipeline)
    #   - ffnvcodec                          (NVIDIA NVENC / NVDEC / CUVID / CUDA)
    #   - AMF                                (AMD Advanced Media Framework)
    #   - libvpl                             (Intel oneVPL, modern QSV)
    #   - OpenCL ICD loader                  (cross-vendor OpenCL filters)
    #   - libva-win32                        (probed but not enabled — Windows
    #                                         path is experimental in upstream)
    # plus FFmpeg native Windows backends (D3D11VA / DXVA2 / Media Foundation)
    # which need no external libs.
    # libnpp is intentionally NOT enabled — requires NVIDIA Performance
    # Primitives proprietary SDK which is not in the BtbN image.
    configure_flags=(
      --prefix=/work/install
      --pkg-config-flags=--static
      --extra-cflags="-I$FFBUILD_PREFIX/include -I$FDK_PREFIX/include"
      --extra-cxxflags="-I$FFBUILD_PREFIX/include -I$FDK_PREFIX/include"
      --extra-ldflags="-L$FFBUILD_PREFIX/lib -L$FDK_PREFIX/lib -pthread"
      --extra-libs="-lgomp"
      --cc="$CC" --cxx="$CXX" --ar="$AR" --ranlib="$RANLIB" --nm="$NM"
      --enable-gpl
      --enable-version3
      --enable-nonfree
      --enable-shared
      --disable-static
      --enable-pic
      --disable-doc
      --disable-debug
      --disable-ffplay
      --disable-w32threads
      --enable-pthreads
      --enable-iconv
      --enable-zlib
      --extra-version=Jellyfin
      # video
      --enable-libx264 --enable-libx265 --enable-libdav1d --enable-libsvtav1
      --enable-libvpx --enable-libaom
      # audio
      --enable-libopus --enable-libvorbis --enable-libmp3lame
      --enable-libfdk-aac
      --enable-libtheora --enable-libopenmpt --enable-libsoxr
      # subtitle / text
      --enable-libass --enable-libfontconfig --enable-libfreetype
      --enable-libfribidi --enable-libharfbuzz
      # container / image / misc
      --enable-libbluray --enable-libwebp --enable-libzimg
      --enable-chromaprint --enable-libsrt --enable-libopenjpeg --enable-libjxl
      --enable-libzvbi
      # GPU: Vulkan + libplacebo
      --enable-vulkan --enable-libplacebo --enable-libshaderc
      # GPU: NVIDIA (NVENC / NVDEC / CUVID / CUDA via ffnvcodec headers)
      --enable-ffnvcodec --enable-cuda --enable-cuda-llvm
      --enable-cuvid --enable-nvdec --enable-nvenc
      # GPU: AMD AMF
      --enable-amf
      # GPU: Intel QSV (modern oneVPL runtime)
      --enable-libvpl
      # NOTE: --enable-opencl conflicts with jellyfin-ffmpeg patches when
      # libvpl/QSV is also enabled (duplicate AV_PIX_FMT_QSV case in
      # libavutil/hwcontext_opencl.c). Windows users get GPU decode/encode
      # via NVENC/AMF/QSV/D3D11VA/DXVA2 + filtering via Vulkan/libplacebo
      # which is the same surface jellyfin-ffmpeg-windows ships, so
      # OpenCL is intentionally omitted here.
      # GPU: native Windows DirectX / Media Foundation backends
      --enable-d3d11va --enable-dxva2 --enable-mediafoundation
    )

    # shellcheck disable=SC2206  # FFBUILD_TARGET_FLAGS is intentionally word-split
    target_flags=( $FFBUILD_TARGET_FLAGS )

    echo "Running FFmpeg configure with ${#configure_flags[@]} flags + ${#target_flags[@]} target flags"
    if ! /work/ffmpeg-src/configure "${target_flags[@]}" "${configure_flags[@]}" \
        > configure.log 2>&1; then
      echo "Configure FAILED. Last 80 lines of configure.log:"
      tail -80 configure.log
      echo "---- ffbuild/config.log tail ----"
      tail -120 ffbuild/config.log 2>/dev/null || true
      exit 1
    fi
    tail -10 configure.log

    make -j"$nproc_count"
    make install

    ls -la /work/install/bin /work/install/lib/pkgconfig
  '

echo "[cross-win] Done. Artifacts in $INSTALL_DIR"
ls -la "$INSTALL_DIR"
ls -la "$INSTALL_DIR/lib/pkgconfig" 2>/dev/null || true
