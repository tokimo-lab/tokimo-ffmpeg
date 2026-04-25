# rsmpeg

This project uses the official [rsmpeg](https://github.com/larksuite/rsmpeg) crate from crates.io, with **no modifications**.

```toml
[dependencies]
rsmpeg = { version = "0.18.0+ffmpeg.8.0", default-features = false, features = ["ffmpeg7_1", "link_system_ffmpeg"] }
```

## Feature Flags

| Feature | Reason |
|---------|--------|
| `ffmpeg7_1` | The patched FFmpeg we build is 7.1.x (libavcodec major = 61). Enables the correct API set without pulling in FFmpeg 8-only bindings. |
| `link_system_ffmpeg` | Finds FFmpeg via `FFMPEG_PKG_CONFIG_PATH` env var, pointing at our custom-built `install/lib/pkgconfig/`. |
| `default-features = false` | Disables the default `ffmpeg8` feature which would target a newer ABI. |

## Key rsmpeg APIs Used

| Module | API | Purpose |
|--------|-----|---------|
| `avformat` | `AVFormatContextInput` | Open/demux input files |
| `avformat` | `AVFormatContextOutput` | Create/mux output files |
| `avcodec` | `AVCodecContext` | Decode/encode |
| `avcodec` | `AVCodec` | Find decoders/encoders by name |
| `avcodec` | `AVPacket` | Compressed data packets |
| `avutil` | `AVFrame` | Decoded video/audio frames |
| `avutil` | `AVHWDeviceContext` | GPU device initialization |
| `avutil` | `AVHWFramesContext` | GPU frame pool management |
| `avutil` | `AVDictionary` | Key-value options |
| `avutil` | `AVChannelLayout` | Audio channel configuration |
| `avfilter` | `AVFilterGraph` | Filter pipeline management |
| `avfilter` | `AVFilterInOut` | Filter graph I/O linkage |
| `ffi` | Raw constants/functions | Direct FFmpeg C API access |

## Thread Safety

rsmpeg's `wrap!` macro includes `unsafe impl Send` for all wrapped types. This is relied upon by our threaded pipeline (sending `AVFrame`/`AVPacket` across decode/filter/encode threads).
