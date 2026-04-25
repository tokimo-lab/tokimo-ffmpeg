# Benchmarks

All benchmarks performed on:
- **GPU:** NVIDIA RTX 4080 16GB, driver 591.74
- **CPU:** AMD (WSL2, kernel 5.15.153.1-microsoft-standard-WSL2)
- **Test file:** 速度与激情6 (2013) — 18 GB MKV, HEVC Main 10 3840×1632 BT.2020/PQ, TrueHD Atmos 7.1
- **FFmpeg:** Jellyfin-patched 7.1.3 (libavcodec.so.61)
- **Pipeline:** 4K HDR (HEVC 10-bit) → H.264 SDR with tonemap_cuda, h264_nvenc preset p1, AAC 5.1

## 10-Seek Playback Simulation

Simulates a user watching a 2-hour movie and seeking 10 times (every 12 minutes).
Each seek transcodes 3 seconds of HDR→SDR video + AAC audio.
CLI spawns a new process per seek. Rust reuses the same process with warm CUDA driver.

| Seek Position | CLI FFmpeg | Rust FFI | Δ |
|---|---|---|---|
| 0s (start) | 854 ms | 798 ms | −7% |
| 720s (12min) | 852 ms | 559 ms | −34% |
| 1440s (24min) | 991 ms | 683 ms | −31% |
| 2160s (36min) | 912 ms | 589 ms | −35% |
| 2880s (48min) | 996 ms | 698 ms | −30% |
| 3600s (1hr) | 904 ms | 573 ms | −37% |
| 4320s (1hr12) | 958 ms | 632 ms | −34% |
| 5040s (1hr24) | 1249 ms | 991 ms | −21% |
| 5760s (1hr36) | 1203 ms | 893 ms | −26% |
| 6480s (1hr48) | 905 ms | 561 ms | −38% |
| **Average** | **982 ms** | **698 ms** | **−29%** |
| **Total** | **9824 ms** | **6976 ms** | **−29%** |

**Rust is 29% faster** across 10 seeks. Best case −38% (6480s), worst case −7% (first seek, CUDA init overhead).

### Where the Time Goes (Rust, warm seek)

```
Rust seek=3600s breakdown:
  CUDA device init:    58 ms  (warm driver cache)
  Input open + seek:   20 ms  (avformat_open_input + avformat_seek_file)
  Filter graph init:    8 ms  (tonemap_cuda pipeline)
  Transcode 3s video: 470 ms  (demux → NVDEC → tonemap_cuda → NVENC → mux)
  Flush + close:       17 ms  (drain encoder, write trailer)
  ────────────────────────────
  Total:              573 ms
```

### Why Rust is Faster

1. **No process spawn** — CLI pays ~30ms fork+exec per seek; Rust stays resident
2. **CUDA driver cache** — First seek: ~120ms init. Subsequent: ~60ms (kernel-level driver cache warm)
3. **Threaded pipeline** — NVDEC/CUDA/NVENC engines overlap via bounded channels
4. **No redundant work** — CLI re-parses args, re-discovers codecs each time

## Direct Transcode (10s, no filter chain)

Pure decode → encode, no tonemap. Shows raw codec performance.

| Codec | CLI FFmpeg | Rust FFI | Δ |
|---|---|---|---|
| HEVC NVENC | 1894 ms | 1728 ms | **−8.8%** |
| H264 NVENC | 1874 ms | 1723 ms | **−8.1%** |
| AV1 NVENC | 2493 ms | 2379 ms | **−4.6%** |
| libx264 (SW) | — | 16840 ms | CPU-bound |

## Single Seek with HDR Tonemap (seek=3600, 3s)

| Metric | CLI FFmpeg | Rust (warm) |
|---|---|---|
| Wall time | 929 ms | 880 ms |
| HW init | ~200 ms | 124 ms |
| Transcode work | ~729 ms | 756 ms |
| **vs CLI** | baseline | **−5.3%** |

## Cold Start Comparison (5 runs, seek=0, 3s)

Both CLI and Rust measured with fresh process invocations:

| Run | CLI FFmpeg | Rust FFI |
|---|---|---|
| 1 | 932 ms | 881 ms |
| 2 | 923 ms | 879 ms |
| 3 | 882 ms | 949 ms |
| 4 | 908 ms | 949 ms |
| 5 | 877 ms | 876 ms |
| **Average** | **904 ms** | **907 ms** |

Cold starts are equivalent. The 29% advantage comes from warm CUDA driver cache in multi-seek scenarios.

## Probe Performance

| Operation | CLI ffprobe | Rust probe |
|---|---|---|
| Probe 18GB MKV | ~120 ms | ~80 ms |
| JSON output | ~130 ms | ~85 ms |

Rust probe is ~35% faster due to no process spawn and direct API access.

## Reproduce

```bash
export LD_LIBRARY_PATH=$PWD/install/lib:$PWD/install/deps
INPUT="速度与激情6 (2013) [tmdbid=82992] - 2160p x265 Atmos.mkv"

# Warm CUDA driver first
./target/release/ffmpeg-tool transcode "$INPUT" /tmp/warmup.mp4 \
  --video-codec h264_nvenc --audio-codec aac --duration 1

# Rust 10-seek
./target/release/ffmpeg-tool bench-seek "$INPUT" \
  --seeks "0,720,1440,2160,2880,3600,4320,5040,5760,6480" \
  --video-codec h264_nvenc --audio-codec aac --duration 3 --bitrate 8000k \
  --audio-bitrate 640k --audio-channels 6 --preset p1 \
  --gop 144 --keyint-min 144 --maxrate 8000k --bufsize 16000k --video-profile high \
  --video-filter "setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc,tonemap_cuda=format=yuv420p:p=bt709:t=bt709:m=bt709:tonemap=bt2390:peak=100:desat=0"

# CLI FFmpeg equivalent (per seek)
time install/bin/ffmpeg -hide_banner -loglevel warning \
  -probesize 1048576 -analyzeduration 2000000 \
  -init_hw_device cuda=cu:0 -filter_hw_device cu \
  -hwaccel cuda -hwaccel_output_format cuda \
  -noautorotate -hwaccel_flags +unsafe_output -threads 1 \
  -ss 3600 -i "$INPUT" -noautoscale -map 0:v:0 -map 0:a:0 \
  -vf "setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc,tonemap_cuda=format=yuv420p:p=bt709:t=bt709:m=bt709:tonemap=bt2390:peak=100:desat=0" \
  -c:v h264_nvenc -preset p1 -profile:v:0 high \
  -b:v 8000k -maxrate 8000k -bufsize 16000k \
  -g:v:0 144 -keyint_min:v:0 144 \
  -c:a aac -b:a 640k -ac 6 -t 3 -y /tmp/cli_test.mp4
```
