# tokimo-ffmpeg 硬件加速支持状态

> 架构版本: v0.2.0 (3-stage composable pipeline)

## 1. 硬件后端支持

| 后端 | 类型 | 状态 | 说明 |
|------|------|------|------|
| **NVIDIA CUDA** | `HwType::Cuda` | 🟢 生产就绪 | RTX 4080 测试通过 |
| **VAAPI** | `HwType::Vaapi` | 🟡 需对应硬件 | HwType + filter 路径完整 |
| **Intel QSV** | `HwType::Qsv` | 🟡 需对应硬件 | VPP 路径完整 |
| **AMD AMF** | `HwType::Amf` | 🟡 需 Windows/AMD | D3D11VA 路径完整 |
| **Apple VideoToolbox** | `HwType::Videotoolbox` | 🟡 需 macOS | 路径完整 |
| **Rockchip RKMPP** | `HwType::Rkmpp` | 🟡 需 ARM 板 | RKRGA + OpenCL 桥接完整 |
| **V4L2M2M** | `HwType::V4l2m2m` | 🟡 需 Linux | 仅解码+编码，无硬件滤镜 |

## 2. 编码器支持

| Codec | NVIDIA | AMD AMF | Intel QSV | VAAPI | VideoToolbox | RKMPP | V4L2M2M |
|-------|--------|---------|-----------|-------|--------------|-------|---------|
| H.264 | h264_nvenc | h264_amf | h264_qsv | h264_vaapi | h264_videotoolbox | h264_rkmpp | h264_v4l2m2m |
| HEVC | hevc_nvenc | hevc_amf | hevc_qsv | hevc_vaapi | hevc_videotoolbox | hevc_rkmpp | — |
| AV1 | av1_nvenc | av1_amf | av1_qsv | av1_vaapi | — | — | — |
| 软件 | libx264 / libx265 / libsvtav1 |

所有编码器名称通过 `infer_hw_from_codec()` 自动推断。

## 3. 滤镜矩阵

| 滤镜 | CUDA | OpenCL | Vulkan | VAAPI | QSV VPP | VideoToolbox | RKRGA |
|------|------|--------|--------|-------|---------|--------------|-------|
| **Scale** | scale_cuda | scale_opencl | scale_vulkan | scale_vaapi | scale_qsv | scale_vt | scale_rkrga |
| **Tonemap** | tonemap_cuda | tonemap_opencl | libplacebo | tonemap_vaapi | vpp_qsv | tonemap_videotoolbox | — |
| **Deinterlace** | yadif_cuda | yadif_opencl | — | deinterlace_vaapi | deinterlace_qsv | yadif_videotoolbox | — |
| **Overlay** | overlay_cuda | overlay_opencl | overlay_vulkan | overlay_vaapi | overlay_qsv | overlay_videotoolbox | overlay_rkrga |
| **Transpose** | transpose_cuda | transpose_opencl | transpose_vulkan | transpose_vaapi | — | transpose_vt | — |

## 4. 管线路径

| 路径类型 | 状态 | 说明 |
|----------|------|------|
| **Full HW (unified)** | ✅ 已测试 | decode+filter+encode 同设备，零拷贝 |
| **Cross-device (hwmap)** | ✅ 已实现 | OpenCL/Vulkan bridge via hwmap=derive_device= |
| **Mixed A (SW decode→HW)** | ✅ 已实现 | FallbackLevel::MixedSwDecode |
| **Mixed B (HW→SW encode)** | ✅ 已实现 | hwdownload → format → SW encode |
| **Pure software** | ✅ 已测试 | libx264 / libx265 / libsvtav1 |

## 5. 跨设备 Interop

| Interop | 说明 |
|---------|------|
| **OpenCL ↔ D3D11VA** | AMD AMF、Intel QSV（Windows）|
| **OpenCL ↔ VAAPI** | Intel iHD、AMD（Linux）|
| **Vulkan ↔ VAAPI** | AMD radeonsi + libplacebo |
| **OpenCL ↔ QSV** | Intel 跨设备色调映射 |
| **OpenCL ↔ RKMPP** | Rockchip ARM 色调映射 |

所有 interop 通过 `build_filter_spec()` 中的 `hwmap=derive_device=` 模式实现。

## 6. Library API

```rust
use ffmpeg_tool::{
    transcode, TranscodeOptions,
    HwType, HwPipeline, HwAccel, FilterBackend, FallbackLevel,
    build_pipeline, resolve_pipeline, resolve_pipeline_with_fallback,
    parse_hw_type, infer_hw_from_codec,
    probe_file, MediaInfo,
};

let pipeline = build_pipeline(
    Some(HwType::Vaapi),
    FilterBackend::OpenCL,
    Some(HwType::Qsv),
    "hevc_qsv",
);
assert_eq!(pipeline.fallback, FallbackLevel::CrossDevice);
```

## 1. 硬件后端支持

| 后端 | Jellyfin | tokimo-ffmpeg | 状态 | 说明 |
|------|----------|---------------|------|------|
| **NVIDIA (NVENC/CUDA)** | ✅ | ✅ 已实现 | 🟢 生产就绪 | RTX 4080 测试通过 |
| **VAAPI** (Intel/AMD) | ✅ | ✅ 类型完整 | 🟡 需对应硬件 | HwType::Vaapi + filter 路径 |
| **Intel QSV** | ✅ | ✅ 类型完整 | 🟡 需对应硬件 | HwType::Qsv + VPP 路径 |
| **AMD AMF** | ✅ | ✅ 类型完整 | 🟡 需 Windows/AMD | HwType::Amf + D3D11VA |
| **Apple VideoToolbox** | ✅ | ✅ 类型完整 | 🟡 需 macOS | HwType::Videotoolbox |
| **Rockchip RKMPP** | ✅ | ✅ 类型完整 | 🟡 需 ARM 板 | HwType::Rkmpp |
| **V4L2M2M** | ✅ | ✅ 类型完整 | 🟡 需 Linux | HwType::V4l2m2m |
| **覆盖率** | 7/7 | **7/7** | **100%** | |

## 2. 编码器支持

| Codec | NVIDIA | AMD AMF | Intel QSV | VAAPI | VideoToolbox | RKMPP | V4L2M2M |
|-------|--------|---------|-----------|-------|--------------|-------|---------|
| H.264 | ✅ h264_nvenc | ✅ h264_amf | ✅ h264_qsv | ✅ h264_vaapi | ✅ h264_videotoolbox | ✅ h264_rkmpp | ✅ h264_v4l2m2m |
| HEVC | ✅ hevc_nvenc | ✅ hevc_amf | ✅ hevc_qsv | ✅ hevc_vaapi | ✅ hevc_videotoolbox | ✅ hevc_rkmpp | — |
| AV1 | ✅ av1_nvenc | ✅ av1_amf | ✅ av1_qsv | ✅ av1_vaapi | — | — | — |
| SW | ✅ libx264/libx265/libsvtav1 |

All encoder names auto-inferred via `infer_hw_from_codec()`. 覆盖率 **100%**.

## 3. 滤镜矩阵

| 滤镜功能 | CUDA | OpenCL | Vulkan | VAAPI | QSV VPP | VideoToolbox | RKRGA |
|----------|------|--------|--------|-------|---------|--------------|-------|
| **Scale** | ✅ scale_cuda | ✅ scale_opencl | ✅ scale_vulkan | ✅ scale_vaapi | ✅ scale_qsv | ✅ scale_vt | ✅ scale_rkrga |
| **Tonemap** | ✅ tonemap_cuda | ✅ tonemap_opencl | ✅ libplacebo | ✅ tonemap_vaapi | ✅ vpp_qsv | ✅ tonemap_videotoolbox | — |
| **Deinterlace** | ✅ yadif_cuda | ✅ yadif_opencl | — | ✅ deinterlace_vaapi | ✅ deinterlace_qsv | ✅ yadif_videotoolbox | — |
| **Overlay** | ✅ overlay_cuda | ✅ overlay_opencl | ✅ overlay_vulkan | ✅ overlay_vaapi | ✅ overlay_qsv | ✅ overlay_videotoolbox | ✅ overlay_rkrga |
| **Transpose** | ✅ transpose_cuda | ✅ transpose_opencl | ✅ transpose_vulkan | ✅ transpose_vaapi | — | ✅ transpose_vt | — |

覆盖率: **100%** (所有 Jellyfin 滤镜名均实现于 `FilterBackend` 方法)

## 4. 管线路径

| 路径类型 | Jellyfin | tokimo-ffmpeg | 说明 |
|----------|----------|---------------|------|
| **Full HW (unified)** | ✅ | ✅ 已测试 | decode+filter+encode 同设备 |
| **Cross-device (hwmap)** | ✅ | ✅ 已实现 | OpenCL/Vulkan bridge (hwmap=derive_device) |
| **Mixed A (SW decode→HW)** | ✅ | ✅ 已实现 | FallbackLevel::MixedSwDecode |
| **Mixed B (HW→SW encode)** | ✅ | ✅ 已实现 | hwdownload → format → SW encode |
| **Pure software** | ✅ | ✅ 已测试 | libx264/libx265/libsvtav1 |

覆盖率: **100%** (所有 5 级回退链)

## 5. 跨后端 Interop

| Interop | Jellyfin | tokimo-ffmpeg | 说明 |
|---------|----------|---------------|------|
| **OpenCL ↔ D3D11VA** | ✅ | ✅ filter graph | AMD AMF, Intel QSV (Windows) |
| **OpenCL ↔ VAAPI** | ✅ | ✅ filter graph | Intel iHD, AMD (Linux) |
| **Vulkan ↔ VAAPI** | ✅ | ✅ filter graph | AMD radeonsi (libplacebo) |
| **OpenCL ↔ QSV** | ✅ | ✅ filter graph | Intel 跨设备 tonemap |
| **OpenCL ↔ RKMPP** | ✅ | ✅ filter graph | Rockchip ARM tonemap |

所有 interop 通过 `build_filter_spec()` 中的 `hwmap=derive_device=` 模式实现。

## 6. Library API

```rust
use ffmpeg_tool::{
    // Core transcode
    transcode, TranscodeOptions,
    // Hardware pipeline (programmatic construction)
    HwType, HwPipeline, HwAccel, FilterBackend, FallbackLevel,
    build_pipeline, resolve_pipeline, resolve_pipeline_with_fallback,
    parse_hw_type, infer_hw_from_codec,
    // Probe
    probe_file, MediaInfo,
};

// Example: programmatic pipeline construction
let pipeline = build_pipeline(
    Some(HwType::Vaapi),      // decode
    FilterBackend::OpenCL,     // filter (cross-device bridge)
    Some(HwType::Qsv),        // encode
    "hevc_qsv",               // encoder name
);
assert_eq!(pipeline.fallback, FallbackLevel::CrossDevice);
```

## 7. 与 Jellyfin 对比总结

| 维度 | Jellyfin | tokimo-ffmpeg v0.2.0 | 覆盖率 |
|------|----------|---------------------|--------|
| HW 后端 | 7 | 7 | **100%** |
| 编码器 | 17+ | 17+ | **100%** |
| 滤镜名 | ~25 | ~25 | **100%** |
| 管线路径 | ~25 | ~25 | **100%** |
| Interop | 5 种 | 5 种 | **100%** |
| 回退链 | 5 级 | 5 级 | **100%** |
| Library API | N/A (C# 内部) | ✅ Rust crate | — |
