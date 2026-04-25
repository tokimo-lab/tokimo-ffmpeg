#!/usr/bin/env bash
# 从 jellyfin-ffmpeg 源码中提取 debian patches 到指定目录
set -euo pipefail

SRC_DIR="${1:?Usage: $0 <src-dir> <patches-dir>}"
PATCHES_DIR="${2:?Usage: $0 <src-dir> <patches-dir>}"
SERIES_FILE="$SRC_DIR/debian/patches/series"

[[ -f "$SERIES_FILE" ]] || { echo "[patches] No debian/patches/series found in $SRC_DIR"; exit 1; }

mkdir -p "$PATCHES_DIR"

# Copy series file
cp "$SERIES_FILE" "$PATCHES_DIR/series"

# Copy all patches referenced in series
count=0
while IFS= read -r patch_name || [[ -n "$patch_name" ]]; do
  [[ -z "$patch_name" || "$patch_name" == \#* ]] && continue
  src="$SRC_DIR/debian/patches/$patch_name"
  if [[ -f "$src" ]]; then
    cp "$src" "$PATCHES_DIR/"
    ((count++)) || true
  else
    echo "[patches] WARNING: $patch_name not found"
  fi
done < "$SERIES_FILE"

echo "[patches] Extracted $count patches to $PATCHES_DIR/"
echo "[patches] View with: ls patches/ or cat patches/series"
