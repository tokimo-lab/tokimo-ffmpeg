# CLI Usage Guide

## Subcommands

```
ffmpeg-tool <COMMAND>

Commands:
  probe       Probe a media file (like ffprobe)
  transcode   Transcode with composable 3-stage hardware pipeline
  bench-seek  Benchmark multi-seek latency
```

## Probe

Analyze a media file and display stream information.

```bash
# Basic probe (human-readable output)
ffmpeg-tool probe video.mkv

# JSON output (like ffprobe -print_format json)
ffmpeg-tool probe --json video.mkv
```

### Output Fields

- **Format:** container name, duration, bitrate, file size, stream count, metadata
- **Video streams:** resolution, pixel format, bit depth, frame rate, aspect ratio, color space/range/primaries/transfer
- **Audio streams:** sample rate, channels, channel layout, sample format, bits per sample
- **Chapters:** ID, start/end time, title

## Transcode

Transcode a media file with composable 3-stage hardware pipeline.

```bash
ffmpeg-tool transcode <INPUT> <OUTPUT> [OPTIONS]
```

### Pipeline Architecture

The pipeline has 3 independent stages, each with its own backend:

```
decode(--decode) → filter(--filter-backend) → encode(--video-codec)
```

- **Decode backend** is auto-inferred from `--video-codec` if `--decode` is omitted
- **Filter backend** defaults to `native` (same device as decode)
- **Encode backend** is determined by `--video-codec` (e.g., `hevc_nvenc` → CUDA)

### Common Examples

```bash
# NVIDIA GPU — auto-inferred from codec name
ffmpeg-tool transcode input.mkv output.mp4 \
  --video-codec h264_nvenc \
  --audio-codec aac

# 4K HDR → SDR with tone mapping
ffmpeg-tool transcode input.mkv output.mp4 \
  --video-codec h264_nvenc \
  --audio-codec aac \
  --preset p1 \
  --bitrate 8000k \
  --maxrate 8000k \
  --bufsize 16000k \
  --audio-bitrate 640k \
  --audio-channels 6 \
  --gop 144 \
  --keyint-min 144 \
  --video-profile high \
  --video-filter "setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc,tonemap_cuda=format=yuv420p:p=bt709:t=bt709:m=bt709:tonemap=bt2390:peak=100:desat=0"

# Software encoding with CRF quality (no HW needed)
ffmpeg-tool transcode input.mkv output.mp4 \
  --video-codec libx264 \
  --audio-codec aac \
  --crf 23

# Explicit decode backend
ffmpeg-tool transcode input.mkv output.mp4 \
  --video-codec hevc_nvenc \
  --decode cuda

# Cross-backend pipeline (future: needs matching hardware)
ffmpeg-tool transcode input.mkv output.mp4 \
  --video-codec hevc_qsv \
  --decode vaapi \
  --filter-backend opencl

# Seek to 1 hour, transcode 30 seconds
ffmpeg-tool transcode input.mkv output.mp4 \
  --video-codec h264_nvenc \
  --seek 3600 \
  --duration 30

# Downscale to 1080p
ffmpeg-tool transcode input.mkv output.mp4 \
  --video-codec h264_nvenc \
  --resolution 1920x1080

# Copy streams (remux only)
ffmpeg-tool transcode input.mkv output.mp4 \
  --video-codec copy \
  --audio-codec copy
```

### All Options

| Option | Default | Description |
|--------|---------|-------------|
| `--video-codec` | `libx264` | Video encoder: `h264_nvenc`, `hevc_nvenc`, `av1_nvenc`, `h264_vaapi`, `hevc_vaapi`, `h264_qsv`, `hevc_qsv`, `h264_amf`, `libx264`, `libx265`, `libsvtav1`, `copy` |
| `--audio-codec` | `aac` | Audio encoder: `aac`, `libopus`, `copy` |
| `--decode` | auto | Decode backend: `cuda`, `vaapi`, `qsv`, `amf`, `videotoolbox`, `rkmpp` |
| `--filter-backend` | `native` | Filter backend: `native`, `opencl`, `vulkan`, `software` |
| `--preset` | `medium` | Encoder preset (e.g., `p1`–`p7` for NVENC, `fast`/`medium`/`slow` for x264) |
| `--crf` | — | Constant rate factor (0–51, quality mode, x264/x265 only) |
| `--bitrate` | — | Target video bitrate (e.g., `5000k`, `10M`) |
| `--maxrate` | — | Maximum bitrate (VBV, e.g., `8000k`) |
| `--bufsize` | — | VBV buffer size (e.g., `16000k`) |
| `--resolution` | input size | Output resolution (e.g., `1920x1080`) |
| `--duration` | full file | Transcode duration limit in seconds |
| `--seek` | 0 | Seek position in seconds (like `-ss`) |
| `--video-filter` | — | Custom GPU filter chain (e.g., tonemap_cuda pipeline) |
| `--video-profile` | — | Encoder profile (e.g., `high`, `main`) |
| `--gop` | 250 | GOP size / keyframe interval |
| `--keyint-min` | — | Minimum keyframe interval |
| `--audio-bitrate` | 128k | Audio bitrate (e.g., `640k`) |
| `--audio-channels` | input ch | Number of audio channels (e.g., 6 for 5.1) |
| `--progress` | off | Show progress during transcoding |

## Bench-Seek

Benchmark multi-seek latency. Simulates a persistent media server handling multiple seek requests.

```bash
ffmpeg-tool bench-seek <INPUT> --seeks <POSITIONS> [OPTIONS]
```

### Example

```bash
# Simulate 5 seeks across a movie (start, 30min, 1hr, 1.5hr, 2hr)
ffmpeg-tool bench-seek movie.mkv \
  --seeks "0,1800,3600,5400,7200" \
  --video-codec h264_nvenc \
  --audio-codec aac \
  --duration 3 \
  --bitrate 8000k \
  --audio-bitrate 640k \
  --preset p1 \
  --video-filter "setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc,tonemap_cuda=format=yuv420p:p=bt709:t=bt709:m=bt709:tonemap=bt2390:peak=100:desat=0" \
  --audio-channels 6 \
  --gop 144 --keyint-min 144 \
  --maxrate 8000k --bufsize 16000k \
  --video-profile high
```

### Output

```
=== Bench-Seek: 5 positions, 3s each ===
...
=== Bench-Seek Results ===
  seek=    0s →     807ms
  seek= 1800s →     648ms
  seek= 3600s →     775ms
  seek= 5400s →     700ms
  seek= 7200s →     895ms
  average:        765ms
  total:         3825ms (5 seeks)
```

## Running with LD_LIBRARY_PATH

When running the binary directly (outside `make`):

```bash
export LD_LIBRARY_PATH=$PWD/install/lib:$PWD/install/deps
./target/release/ffmpeg-tool probe video.mkv
```

Or inline:

```bash
LD_LIBRARY_PATH=install/lib:install/deps ./target/release/ffmpeg-tool transcode in.mkv out.mp4
```
