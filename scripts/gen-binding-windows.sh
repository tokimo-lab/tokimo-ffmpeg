#!/usr/bin/env bash
# Generate rusty_ffmpeg-compatible binding.rs for the Windows GNU ABI
# from the headers shipped in `install-windows/include`.
#
# Why: rusty_ffmpeg's build.rs would otherwise run bindgen on the
# Windows runner. Windows-latest's clang ships with an MSVC default
# target, which mismatches the mingw GCC ABI we just built. Even if we
# coax it to `-target x86_64-w64-mingw32`, bindgen on Windows tends to
# emit accessor methods (and drop fields) for AVFormatContext members
# whose type is a pointer to a forward-declared composite (`pb`,
# `iformat`, `oformat`, `streams`, `nb_streams`, `metadata`) under
# _WIN32 / __MINGW32__ predefines, which rsmpeg 0.18 source code can't
# compile against. Generating the binding here on Linux against mingw
# headers — but with the rusty_ffmpeg invocation flags — produces a
# binding.rs that both compiles and matches the mingw ABI.
#
# Outputs:
#   ffmpeg-binding-windows/binding.rs
#
# Inputs:
#   install-windows/include/lib{avcodec,avdevice,avfilter,avformat,
#                                avutil,swresample,swscale}/*.h
#   bindgen-cli  (cargo install bindgen-cli, kept on PATH)
#   gcc-mingw-w64-x86-64  (apt-get install) — provides clang sysroot
#   clang  (apt-get install)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

INSTALL_DIR="${INSTALL_DIR:-$ROOT_DIR/install-windows}"
OUT_DIR="${OUT_DIR:-$ROOT_DIR/ffmpeg-binding-windows}"
INCLUDE_DIR="$INSTALL_DIR/include"

if [[ ! -d "$INCLUDE_DIR" ]]; then
  echo "[gen-binding] ERROR: $INCLUDE_DIR not found. Run cross-build-windows.sh first." >&2
  exit 1
fi

if ! command -v bindgen >/dev/null 2>&1; then
  echo "[gen-binding] ERROR: bindgen not on PATH. Install with: cargo install --locked bindgen-cli" >&2
  exit 1
fi

# Locate mingw-w64 sysroot. Ubuntu's gcc-mingw-w64-x86-64 installs at
# /usr/x86_64-w64-mingw32/include for the C runtime + Win32 API headers.
MINGW_SYSROOT="${MINGW_SYSROOT:-/usr/x86_64-w64-mingw32}"
if [[ ! -d "$MINGW_SYSROOT/include" ]]; then
  echo "[gen-binding] ERROR: mingw sysroot $MINGW_SYSROOT/include missing. apt-get install gcc-mingw-w64-x86-64." >&2
  exit 1
fi

mkdir -p "$OUT_DIR"
WRAPPER="$OUT_DIR/wrapper.h"

# Header set is copied from rusty_ffmpeg 0.16.7's build.rs HEADERS array
# (headers commented out in upstream — d3d11va, dxva2, qsv, vdpau,
# videotoolbox, xvmc, hwcontext_*) are intentionally omitted here too.
cat >"$WRAPPER" <<'EOF'
#include "libavcodec/ac3_parser.h"
#include "libavcodec/adts_parser.h"
#include "libavcodec/avcodec.h"
#include "libavcodec/avdct.h"
#include "libavcodec/avfft.h"
#include "libavcodec/bsf.h"
#include "libavcodec/codec.h"
#include "libavcodec/codec_desc.h"
#include "libavcodec/codec_id.h"
#include "libavcodec/codec_par.h"
#include "libavcodec/defs.h"
#include "libavcodec/dirac.h"
#include "libavcodec/dv_profile.h"
#include "libavcodec/jni.h"
#include "libavcodec/mediacodec.h"
#include "libavcodec/packet.h"
#include "libavcodec/version.h"
#include "libavcodec/version_major.h"
#include "libavcodec/vorbis_parser.h"
#include "libavdevice/avdevice.h"
#include "libavdevice/version.h"
#include "libavdevice/version_major.h"
#include "libavfilter/avfilter.h"
#include "libavfilter/buffersink.h"
#include "libavfilter/buffersrc.h"
#include "libavfilter/version.h"
#include "libavfilter/version_major.h"
#include "libavformat/avformat.h"
#include "libavformat/avio.h"
#include "libavformat/version.h"
#include "libavformat/version_major.h"
#include "libavutil/adler32.h"
#include "libavutil/aes.h"
#include "libavutil/aes_ctr.h"
#include "libavutil/ambient_viewing_environment.h"
#include "libavutil/attributes.h"
#include "libavutil/audio_fifo.h"
#include "libavutil/avassert.h"
#include "libavutil/avconfig.h"
#include "libavutil/avstring.h"
#include "libavutil/avutil.h"
#include "libavutil/base64.h"
#include "libavutil/blowfish.h"
#include "libavutil/bprint.h"
#include "libavutil/bswap.h"
#include "libavutil/buffer.h"
#include "libavutil/camellia.h"
#include "libavutil/cast5.h"
#include "libavutil/channel_layout.h"
#include "libavutil/common.h"
#include "libavutil/cpu.h"
#include "libavutil/crc.h"
#include "libavutil/csp.h"
#include "libavutil/des.h"
#include "libavutil/detection_bbox.h"
#include "libavutil/dict.h"
#include "libavutil/display.h"
#include "libavutil/dovi_meta.h"
#include "libavutil/downmix_info.h"
#include "libavutil/encryption_info.h"
#include "libavutil/error.h"
#include "libavutil/eval.h"
#include "libavutil/executor.h"
#include "libavutil/ffversion.h"
#include "libavutil/fifo.h"
#include "libavutil/file.h"
#include "libavutil/film_grain_params.h"
#include "libavutil/frame.h"
#include "libavutil/hash.h"
#include "libavutil/hdr_dynamic_metadata.h"
#include "libavutil/hdr_dynamic_vivid_metadata.h"
#include "libavutil/hmac.h"
#include "libavutil/hwcontext.h"
#include "libavutil/imgutils.h"
#include "libavutil/intfloat.h"
#include "libavutil/intreadwrite.h"
#include "libavutil/lfg.h"
#include "libavutil/log.h"
#include "libavutil/lzo.h"
#include "libavutil/macros.h"
#include "libavutil/mastering_display_metadata.h"
#include "libavutil/mathematics.h"
#include "libavutil/md5.h"
#include "libavutil/mem.h"
#include "libavutil/motion_vector.h"
#include "libavutil/murmur3.h"
#include "libavutil/opt.h"
#include "libavutil/parseutils.h"
#include "libavutil/pixdesc.h"
#include "libavutil/pixelutils.h"
#include "libavutil/pixfmt.h"
#include "libavutil/random_seed.h"
#include "libavutil/rational.h"
#include "libavutil/rc4.h"
#include "libavutil/replaygain.h"
#include "libavutil/ripemd.h"
#include "libavutil/samplefmt.h"
#include "libavutil/sha.h"
#include "libavutil/sha512.h"
#include "libavutil/spherical.h"
#include "libavutil/stereo3d.h"
#include "libavutil/tea.h"
#include "libavutil/threadmessage.h"
#include "libavutil/time.h"
#include "libavutil/timecode.h"
#include "libavutil/timestamp.h"
#include "libavutil/tree.h"
#include "libavutil/twofish.h"
#include "libavutil/tx.h"
#include "libavutil/uuid.h"
#include "libavutil/version.h"
#include "libavutil/video_enc_params.h"
#include "libavutil/video_hint.h"
#include "libavutil/xtea.h"
#include "libswresample/swresample.h"
#include "libswresample/version.h"
#include "libswresample/version_major.h"
#include "libswscale/swscale.h"
#include "libswscale/version.h"
#include "libswscale/version_major.h"
EOF

OUTPUT="$OUT_DIR/binding.rs"

# Flags mirror rusty_ffmpeg's bindgen::builder() invocation in
# build.rs (generate_bindings()):
#   .impl_debug(true)                                   → --impl-debug
#   .rust_target(stable(68, 0))                         → --rust-target=1.68
#   .blocklist_type("__mingw_ldbl_type_t")              → --blocklist-type
#   .prepend_enum_name(false)                           → --no-prepend-enum-name
#   FilterCargoCallbacks ignoring FP_NAN/INFINITE/...   → --blocklist-item
#   .clang_arg("-I<ffmpeg_include_dir>")                → -- -I...
#
# The mingw -target tells clang to use the MSVCRT-flavor _WIN32 macro
# layout that mingw GCC sees, so the resulting binding.rs's struct
# field layout matches what FFmpeg's headers expand to under our cross
# toolchain. The -isystem points clang at the mingw libc / Win32 API
# headers (Ubuntu's gcc-mingw-w64-x86-64 package).
bindgen "$WRAPPER" \
  --output "$OUTPUT" \
  --rust-target=1.68 \
  --impl-debug \
  --no-prepend-enum-name \
  --blocklist-type='__mingw_ldbl_type_t' \
  --blocklist-item='FP_NAN' \
  --blocklist-item='FP_INFINITE' \
  --blocklist-item='FP_ZERO' \
  --blocklist-item='FP_SUBNORMAL' \
  --blocklist-item='FP_NORMAL' \
  -- \
  -target x86_64-w64-mingw32 \
  -isystem "$MINGW_SYSROOT/include" \
  -I "$INCLUDE_DIR"

lines=$(wc -l < "$OUTPUT")
bytes=$(wc -c < "$OUTPUT")
echo "[gen-binding] Wrote $OUTPUT ($lines lines, $bytes bytes)"
