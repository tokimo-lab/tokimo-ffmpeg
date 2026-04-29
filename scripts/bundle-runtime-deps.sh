#!/usr/bin/env bash
# bundle-runtime-deps.sh – copy third-party runtime .so deps into install/lib/
# and rewrite RUNPATH so the bundle is self-contained.
#
# Usage: ./scripts/bundle-runtime-deps.sh [INSTALL_DIR]
#   INSTALL_DIR defaults to "install"
set -euo pipefail

INSTALL_DIR="${1:-install}"
LIB_DIR="$(realpath "$INSTALL_DIR/lib")"

# ── Skip-list: glibc / base libs always present on target distros ──
SKIP_PATTERNS=(
  "ld-linux"
  "libc.so."
  "libdl.so."
  "libm.so."
  "libpthread.so."
  "librt.so."
  "libgcc_s.so."
  "libstdc++.so."
  "libresolv.so."
  "libutil.so."
  "libnsl.so."
  "libcrypt.so."
  "libz.so."
  "libbz2.so."
  "liblzma.so."
  "libselinux.so."
  "libpcre"
  "linux-vdso"
  "libanl"
)

should_skip() {
  local basename="$1"
  for pat in "${SKIP_PATTERNS[@]}"; do
    case "$basename" in
      ${pat}*) return 0 ;;
    esac
  done
  return 1
}

# ── Collect ldd output for a binary/lib, emit resolved paths ──
collect_deps() {
  local target="$1"
  # ldd prints lines like:
  #   libfoo.so.1 => /usr/lib/x86_64-linux-gnu/libfoo.so.1 (0x...)
  #   /lib64/ld-linux-x86-64.so.2 (0x...)
  ldd "$target" 2>/dev/null \
    | awk '/=>/ { print $3 }' \
    | grep -v '^$' \
    | grep -v 'not found' \
    || true
}

# ── Initial set: deps of ffmpeg binary + every libav*/libsw*/libpostproc ──
declare -A queued   # path -> 1 (already scheduled)
declare -a queue=()

enqueue() {
  local path="$1"
  if [[ -z "${queued[$path]+x}" ]]; then
    queued["$path"]=1
    queue+=("$path")
  fi
}

targets=()
[[ -f "$INSTALL_DIR/bin/ffmpeg" ]]  && targets+=("$INSTALL_DIR/bin/ffmpeg")
[[ -f "$INSTALL_DIR/bin/ffprobe" ]] && targets+=("$INSTALL_DIR/bin/ffprobe")
for so in "$INSTALL_DIR"/lib/libav*.so* \
          "$INSTALL_DIR"/lib/libsw*.so* \
          "$INSTALL_DIR"/lib/libpostproc.so*; do
  [[ -f "$so" && ! -L "$so" ]] && targets+=("$so")
done

for t in "${targets[@]}"; do
  while IFS= read -r dep; do
    [[ -n "$dep" ]] && enqueue "$dep"
  done < <(collect_deps "$t")
done

# ── BFS: walk transitive deps until fixed point ──
bundled=0
idx=0
while (( idx < ${#queue[@]} )); do
  dep="${queue[$idx]}"
  (( idx++ ))

  [[ -f "$dep" ]] || continue
  basename="$(basename "$dep")"

  # skip glibc/base libs
  if should_skip "$basename"; then
    continue
  fi

  # skip if realpath is already inside our lib dir
  real="$(realpath "$dep")"
  real_dir="$(dirname "$real")"
  if [[ "$real_dir" == "$LIB_DIR" ]]; then
    continue
  fi

  # skip if already bundled (file with same basename already in lib dir)
  if [[ -e "$LIB_DIR/$basename" ]]; then
    # still chase its deps in case it was pre-existing FFmpeg lib
    while IFS= read -r trans; do
      [[ -n "$trans" ]] && enqueue "$trans"
    done < <(collect_deps "$dep")
    continue
  fi

  echo "  + $basename  (from $dep)"
  cp -L "$dep" "$LIB_DIR/$basename"
  (( bundled++ ))

  # chase transitive deps of the just-copied lib
  while IFS= read -r trans; do
    [[ -n "$trans" ]] && enqueue "$trans"
  done < <(collect_deps "$dep")
done

echo ""
echo "Bundled $bundled third-party runtime libs into $INSTALL_DIR/lib/"
echo ""

# ── Rewrite RUNPATH so libs find each other at $ORIGIN ──
echo "Rewriting RUNPATH on bundled libs to \$ORIGIN …"
for f in "$INSTALL_DIR"/lib/*.so*; do
  [[ -L "$f" ]] && continue
  [[ -f "$f" ]] || continue
  patchelf --set-rpath '$ORIGIN' "$f" || true
done

echo "Rewriting RUNPATH on ffmpeg/ffprobe to \$ORIGIN/../lib …"
for bin in "$INSTALL_DIR/bin/ffmpeg" "$INSTALL_DIR/bin/ffprobe"; do
  [[ -f "$bin" ]] || continue
  patchelf --set-rpath '$ORIGIN/../lib' "$bin" || true
done

echo ""
echo "── Final $INSTALL_DIR/lib/ contents ──"
ls -la "$INSTALL_DIR/lib/"
