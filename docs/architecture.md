# Architecture

## Overview

tokimo-ffmpeg is a Rust library + CLI tool that directly links to a patched FFmpeg via FFI (rsmpeg bindings). It implements a composable 3-stage hardware pipeline architecture — each stage independently configurable with automatic cross-device interop.

Key design principles:
- **Library-first** — `src/lib.rs` exports all types; CLI is just a test harness
- **Composable pipeline** — decode/filter/encode stages can each use different HW backends
- **Cross-device interop** — OpenCL/Vulkan bridges via `hwmap=derive_device=`
- **5-level fallback** — graceful degradation from full-HW to pure software
- **Zero-copy GPU** — frames stay on GPU through entire decode→filter→encode chain

## Module Structure

```
src/
├── lib.rs                   # Library entry: re-exports public API
├── main.rs                  # CLI test harness (clap): probe / transcode / bench-seek
├── probe.rs        (615L)   # Media probing (like ffprobe), JSON/struct output
└── transcode/
    ├── mod.rs      (989L)   # Orchestrator: pipeline init → thread spawn → mux
    ├── hw.rs       (499L)   # 3-stage hardware abstraction layer
    ├── filter.rs   (363L)   # Filter graph construction (unified + cross-device)
    ├── encode.rs   (167L)   # Encoder setup, options, flush
    └── pipeline.rs (276L)   # Threaded pipeline (decode / filter+encode / audio)
```

## Library API

```rust
// Public exports from lib.rs
pub use transcode::{transcode, TranscodeOptions};
pub use transcode::hw::{
    HwType, HwPipeline, HwAccel, FilterBackend, FallbackLevel,
    resolve_pipeline, build_pipeline, resolve_pipeline_with_fallback,
    parse_hw_type, infer_hw_from_codec,
};
pub use probe::{probe_file, MediaInfo};
```

## 3-Stage Composable Pipeline

### Design

```
┌──────────────┐       ┌──────────────┐       ┌──────────────┐
│   DECODE     │       │   FILTER     │       │   ENCODE     │
│              │       │              │       │              │
│ HwType::Cuda │──────→│ FilterBackend│──────→│ HwType::Qsv  │
│ HwType::Vaapi│  hwmap│ ::Native     │  hwmap│ HwType::Cuda │
│ HwType::Qsv  │       │ ::OpenCL     │       │ None (SW)    │
│ None (SW)    │       │ ::Vulkan     │       │              │
│              │       │ ::Software   │       │              │
└──────────────┘       └──────────────┘       └──────────────┘
```

### Key Types

```rust
// Device type (shared across decode/encode)
pub enum HwType {
    Cuda,          // NVIDIA: NVDEC + NVENC
    Vaapi,         // AMD/Intel: VAAPI
    Qsv,           // Intel: Quick Sync Video
    Amf,           // AMD: Advanced Media Framework (D3D11VA)
    Videotoolbox,  // Apple: VideoToolbox
    Rkmpp,         // Rockchip: Media Process Platform
    V4l2m2m,       // Linux: Video4Linux2
}

// Filter backend (may differ from decode/encode device)
pub enum FilterBackend {
    Native,    // Same device: scale_cuda, scale_vaapi, etc.
    OpenCL,    // Cross-device bridge: hwmap → scale_opencl → hwmap
    Vulkan,    // Cross-device bridge: hwmap → libplacebo → hwmap
    Software,  // CPU: hwdownload → scale → format
}

// Fully resolved pipeline configuration
pub struct HwPipeline {
    pub decode: Option<HwType>,      // None = software decode
    pub filter: FilterBackend,
    pub encode: Option<HwType>,      // None = software encode
    pub encoder_name: String,        // e.g. "hevc_nvenc"
    pub fallback: FallbackLevel,     // which fallback level was used
}

pub enum FallbackLevel {
    FullHw,          // decode+filter+encode same device (zero-copy)
    CrossDevice,     // hwmap between different devices
    MixedSwDecode,   // SW decode → hwupload → HW encode
    MixedHwDownload, // HW decode → hwdownload → SW/HW encode
    Software,        // pure CPU
}
```

### Pipeline Resolution

```rust
// Auto-infer from codec name
resolve_pipeline("hevc_nvenc", None, None)
// → HwPipeline { decode: Cuda, filter: Native, encode: Cuda, fallback: FullHw }

// Explicit cross-device
resolve_pipeline("hevc_qsv", Some("vaapi"), Some("opencl"))
// → HwPipeline { decode: Vaapi, filter: OpenCL, encode: Qsv, fallback: CrossDevice }

// Programmatic (for library callers)
build_pipeline(Some(HwType::Vaapi), FilterBackend::OpenCL, Some(HwType::Qsv), "hevc_qsv")
```

## Cross-Device Filter Graphs

### Unified (Native) — Zero-Copy

```
buffer(CUDA) → scale_cuda=format=nv12 → buffersink(CUDA)
```

All filters run on the same device. No memory transfer.

### OpenCL Bridge — AMD/Intel Pattern

```
buffer(VAAPI) → hwmap=derive_device=opencl → scale_opencl=format=nv12 → hwmap=derive_device=vaapi → buffersink(VAAPI)
```

FFmpeg's `hwmap` derives an OpenCL device from the VAAPI device, maps frames to OpenCL address space, runs the filter, then maps back. Used for:
- AMD AMF: `d3d11va → opencl → amf`
- Intel QSV Linux: `vaapi → opencl → qsv`
- Intel QSV Windows: `d3d11va → opencl → qsv`
- Intel iHD tonemap: `vaapi → opencl → vaapi`
- Rockchip: `drm_prime → opencl → rkmpp`

### Vulkan Bridge — AMD VAAPI Pattern

```
buffer(VAAPI) → hwmap=derive_device=vulkan → libplacebo=format=nv12 → hwmap=derive_device=vaapi → buffersink(VAAPI)
```

Used for AMD radeonsi (kernel ≥5.1) with libplacebo tonemap.

### Software Fallback

```
buffer(CUDA) → hwdownload → format=nv12 → scale=1920:1080 → buffersink(SW)
```

Pulls frames to CPU for software processing when no HW filter is available.

## Thread Architecture

### Threaded GPU Pipeline

```
Main thread:    demux (read_packet) ─────────────────── mux (write_frame)
                  │                                        ▲
                  ▼                                        │
Decode thread:  recv pkt → HW decode → send frame          │
                                        │                  │
                                        ▼                  │
Filter+Enc:    recv frame → [hwmap] → filter → encode ─────┘
Audio thread:   decode → aformat → aac encode ─────────────┘
```

**Channel types:**
- `sync_channel(4)` — demux → decode (bounded, backpressure)
- `sync_channel(2)` — decode → filter+encode (bounded, GPU memory control)
- `mpsc::channel()` — encode → mux (unbounded, low latency)

### GPU Engine Overlap

While NVDEC decodes frame N+2, CUDA filters frame N+1, and NVENC encodes frame N — all three GPU engines work in parallel via the bounded channel pipeline.

## 5-Level Fallback Chain

Fallback logic applied in `resolve_pipeline_with_fallback()`:

```
Level 1: Full HW (unified)
  Cuda decode → scale_cuda → nvenc encode
  ↓ device init fails

Level 2: Cross-device (hwmap bridge)
  VAAPI decode → hwmap → OpenCL filter → hwmap → QSV encode
  ↓ bridge not available

Level 3: Mixed A (SW decode → HW encode)
  SW decode → hwupload → HW filter → HW encode
  ↓ HW filter not available

Level 4: Mixed B (HW decode → SW filter)
  HW decode → hwdownload → SW filter → HW encode
  ↓ HW encode fails

Level 5: Pure software
  SW decode → SW filter → libx264/libx265/libsvtav1
```

## Memory Model

- **GPU frames stay on GPU** in unified pipeline — no GPU↔CPU transfer
- **Bounded channels** prevent unbounded memory growth
- **FilterPipeline lifetime** — filter graph references transmuted to `'static` for thread movement (safe: owning Vec outlives thread::scope)
- **AVFrame/AVPacket are Send** — rsmpeg includes `unsafe impl Send` for FFI types

## CLI FFmpeg Parity

All critical FFmpeg CLI flags are replicated:

| CLI Flag | Implementation | Purpose |
|---|---|---|
| `-init_hw_device cuda=cu:0` | `HwAccel::try_init(HwType::Cuda)` | Initialize GPU device |
| `-hwaccel cuda` | `dec_ctx.set_hw_device_ctx()` | Enable hardware decoding |
| `-hwaccel_output_format cuda` | `pix_fmt = AV_PIX_FMT_CUDA` | Keep frames on GPU |
| `-hwaccel_flags +unsafe_output` | `AV_HWACCEL_FLAG_UNSAFE_OUTPUT` | Skip format validation |
| `-vf hwmap=derive_device=opencl` | `build_filter_spec()` | Cross-device bridge |
| `-threads 1` | `thread_count = 1` | GPU handles parallelism |
| `-probesize 1048576` | Input dict option | Fast format detection |
| `-avoid_negative_ts disabled` | `AVFMT_AVOID_NEG_TS_DISABLED` | Preserve timestamps |
| `-map_metadata -1` | `av_dict_free(metadata)` | Strip metadata |
