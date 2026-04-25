# Test Scripts

## Quick Validation

```bash
# Set library path (needed for all commands below)
export LD_LIBRARY_PATH=$PWD/install/lib:$PWD/install/deps

# Verify FFmpeg libraries are accessible
./target/release/ffmpeg-tool probe --help

# Probe a test file
./target/release/ffmpeg-tool probe "test-video.mkv"

# Probe with JSON output
./target/release/ffmpeg-tool probe --json "test-video.mkv" | python3 -m json.tool
```

## Transcode Tests

### Test 1: Basic GPU Transcode (H.264 NVENC)

```bash
./target/release/ffmpeg-tool transcode "input.mkv" /tmp/test_basic.mp4 \
  --video-codec h264_nvenc \
  --audio-codec aac \
  --duration 10
```

**Expected:** HW auto-inferred as CUDA from `h264_nvenc`. Pipeline: `Cuda → Native → Cuda(h264_nvenc)`. Completes in ~2s.

### Test 2: HDR → SDR Tone Mapping (Full Jellyfin Pipeline)

```bash
./target/release/ffmpeg-tool transcode "4k-hdr-input.mkv" /tmp/test_tonemap.mp4 \
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
  --video-filter "setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc,tonemap_cuda=format=yuv420p:p=bt709:t=bt709:m=bt709:tonemap=bt2390:peak=100:desat=0" \
  --seek 3600 \
  --duration 3
```

**Expected:** Pipeline `Cuda → Native → Cuda(h264_nvenc)`. Output in BT.709 color space, ~880ms warm.

### Test 3: Software Encoding (CPU)

```bash
./target/release/ffmpeg-tool transcode "input.mkv" /tmp/test_cpu.mp4 \
  --video-codec libx264 \
  --audio-codec aac \
  --crf 23 \
  --duration 5
```

**Expected:** No HW init (libx264 infers software pipeline). Slower (~12fps for 4K).

### Test 4: Seek + Duration

```bash
./target/release/ffmpeg-tool transcode "input.mkv" /tmp/test_seek.mp4 \
  --video-codec h264_nvenc \
  --seek 7200 \
  --duration 5
```

**Expected:** Output starts at the 2-hour mark, lasts 5 seconds.

### Test 5: Explicit Decode Backend

```bash
./target/release/ffmpeg-tool transcode "input.mkv" /tmp/test_explicit.mp4 \
  --video-codec hevc_nvenc \
  --decode cuda \
  --duration 10
```

**Expected:** Same as auto-infer, but `--decode cuda` is explicit.

### Test 6: Cross-Backend (future, needs VAAPI/QSV hardware)

```bash
./target/release/ffmpeg-tool transcode "input.mkv" /tmp/test_cross.mp4 \
  --video-codec hevc_qsv \
  --decode vaapi \
  --filter-backend opencl \
  --duration 10
```

**Expected:** Fails on NVIDIA-only systems with "VAAPI decode device not available".

### Test 7: Stream Copy (Remux)

```bash
./target/release/ffmpeg-tool transcode "input.mkv" /tmp/test_remux.mp4 \
  --video-codec copy \
  --audio-codec copy \
  --duration 30
```

**Expected:** Near-instant (just copies packets, no encoding).

## Benchmark Tests

### Multi-Seek Benchmark

```bash
./target/release/ffmpeg-tool bench-seek "4k-hdr-input.mkv" \
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

**Expected output:**
```
=== Bench-Seek Results ===
  seek=    0s →     ~800ms
  seek= 1800s →     ~650ms
  seek= 3600s →     ~775ms
  seek= 5400s →     ~700ms
  seek= 7200s →     ~895ms
  average:        ~765ms
  total:         ~3825ms (5 seeks)
```

### Compare with CLI FFmpeg

```bash
# CLI equivalent for single seek:
time install/bin/ffmpeg -hide_banner -loglevel warning \
  -probesize 1048576 -analyzeduration 2000000 \
  -init_hw_device cuda=cu:0 -filter_hw_device cu \
  -hwaccel cuda -hwaccel_output_format cuda \
  -noautorotate -hwaccel_flags +unsafe_output \
  -threads 1 \
  -ss 3600 -i "input.mkv" \
  -noautoscale -map 0:v:0 -map 0:a:0 \
  -vf "setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc,tonemap_cuda=format=yuv420p:p=bt709:t=bt709:m=bt709:tonemap=bt2390:peak=100:desat=0" \
  -c:v h264_nvenc -preset p1 -profile:v:0 high \
  -b:v 8000k -maxrate 8000k -bufsize 16000k \
  -g:v:0 144 -keyint_min:v:0 144 \
  -c:a aac -b:a 640k -ac 6 \
  -t 3 -y /tmp/cli_test.mp4
```

**Expected:** CLI takes ~929ms; Rust takes ~880ms warm, ~765ms average in multi-seek.

## Output Validation

After any transcode test, validate the output:

```bash
# Check output with our probe tool
./target/release/ffmpeg-tool probe /tmp/test_tonemap.mp4

# Verify with ffprobe (cross-reference)
install/bin/ffprobe -hide_banner /tmp/test_tonemap.mp4

# Check file plays correctly
ffplay /tmp/test_tonemap.mp4  # or any media player
```

**What to verify:**
- Duration matches expected (seek + duration args)
- Video codec matches (H.264 High for h264_nvenc)
- Color space is BT.709 (after tonemap)
- Audio is AAC with correct channel count
- No artifacts or corruption in output
