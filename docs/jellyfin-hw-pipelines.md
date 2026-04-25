# Jellyfin 硬件加速管线完整分析

> 基于 jellyfin/jellyfin `main` 分支源码分析  
> 核心文件: `MediaBrowser.Controller/MediaEncoding/EncodingHelper.cs` (7,851 行)

---

## 1. 硬件后端总览

Jellyfin 定义了 7 种硬件加速类型 (`HardwareAccelerationType` 枚举):

| # | 后端 | 枚举值 | 平台 | 说明 |
|---|------|--------|------|------|
| 0 | Software | `none` | 全平台 | 纯 CPU 编解码 |
| 1 | **AMD AMF** | `amf` | Windows | AMD Advanced Media Framework |
| 2 | **Intel QSV** | `qsv` | Windows/Linux | Intel Quick Sync Video |
| 3 | **NVIDIA NVENC** | `nvenc` | Windows/Linux | NVIDIA 编解码加速 |
| 4 | **V4L2M2M** | `v4l2m2m` | Linux | Video4Linux2 通用接口 |
| 5 | **VAAPI** | `vaapi` | Linux | Video Acceleration API (Intel/AMD) |
| 6 | **VideoToolbox** | `videotoolbox` | macOS | Apple 硬件加速框架 |
| 7 | **RKMPP** | `rkmpp` | Linux ARM | Rockchip 媒体处理平台 |

---

## 2. 编解码器支持矩阵

### 2.1 编码器 (Encoder)

| Codec | NVIDIA | AMD AMF | Intel QSV | VAAPI | VideoToolbox | RKMPP | V4L2M2M |
|-------|--------|---------|-----------|-------|--------------|-------|---------|
| H.264 | h264_nvenc | h264_amf | h264_qsv | h264_vaapi | h264_videotoolbox | h264_rkmpp | h264_v4l2m2m |
| HEVC  | hevc_nvenc | hevc_amf | hevc_qsv | hevc_vaapi | hevc_videotoolbox | hevc_rkmpp | — |
| AV1   | av1_nvenc  | av1_amf  | av1_qsv  | av1_vaapi  | — | — | — |
| MJPEG | — | — | mjpeg_qsv | mjpeg_vaapi | mjpeg_videotoolbox | mjpeg_rkmpp | — |

### 2.2 解码器 (Decoder)

| Codec | NVIDIA (cuvid) | Intel QSV | VAAPI | VideoToolbox | RKMPP |
|-------|----------------|-----------|-------|--------------|-------|
| H.264 | h264_cuvid | h264_qsv | h264_vaapi | h264_videotoolbox | h264_rkmpp |
| HEVC  | hevc_cuvid | hevc_qsv | hevc_vaapi | hevc_videotoolbox | hevc_rkmpp |
| VP9   | vp9_cuvid  | vp9_qsv  | — | — | — |
| AV1   | av1_cuvid  | av1_qsv  | av1_vaapi | — | — |
| VP8   | vp8_cuvid  | vp8_qsv  | — | — | — |
| MPEG2 | mpeg2_cuvid | mpeg2_qsv | — | — | — |
| MPEG4 | mpeg4_cuvid | — | — | — | — |
| VC1   | vc1_cuvid  | vc1_qsv  | — | — | — |

> AMD AMF 不提供专用解码器，使用 d3d11va 硬件加速器解码。

---

## 3. 核心架构：三段式混合管线

```
┌──────────┐     ┌──────────┐     ┌──────────┐
│  DECODE  │ ──→ │  FILTER  │ ──→ │  ENCODE  │
│ (解码器)  │     │ (滤镜链)  │     │ (编码器)  │
└──────────┘     └──────────┘     └──────────┘
```

**关键设计**：三段可以来自不同硬件后端，通过 `hwmap` / `hwupload` / `hwdownload` 实现跨后端 interop。

---

## 4. 滤镜矩阵

| 滤镜功能 | CUDA | OpenCL | Vulkan | VAAPI | QSV VPP | VideoToolbox | RKRGA |
|----------|------|--------|--------|-------|---------|--------------|-------|
| **Scale** | scale_cuda | scale_opencl | libplacebo / scale_vulkan | scale_vaapi | vpp_qsv | scale_vt | scale_rkrga |
| **Tonemap** | tonemap_cuda | tonemap_opencl | libplacebo | tonemap_vaapi | vpp_qsv (Gen12+) | tonemap_videotoolbox | — |
| **Deinterlace** | yadif_cuda / bwdif_cuda | yadif_opencl / bwdif_opencl | — | deinterlace_vaapi | deinterlace_qsv | yadif_videotoolbox / bwdif_videotoolbox | — |
| **Overlay** | overlay_cuda | overlay_opencl | overlay_vulkan | overlay_vaapi | overlay_qsv | overlay_videotoolbox | overlay_rkrga |
| **Transpose** | transpose_cuda | transpose_opencl | transpose_vulkan + flip_vulkan | transpose_vaapi | — | transpose_vt | — |

> OpenCL 和 Vulkan **不是独立管线**，而是作为跨后端的**滤镜桥梁**。

---

## 5. 全部管线路径（约 25+ 种）

### 5.1 NVIDIA 管线 (4 种)

| 路径 | Decode | Filter | Encode | 说明 |
|------|--------|--------|--------|------|
| **首选** | cuda (nvdec/cuvid) | CUDA 滤镜 | nvenc | 全 GPU，零拷贝 |
| 混合 A | SW (libavcodec) | hwupload → CUDA 滤镜 | nvenc | CPU 解码 → GPU 滤镜/编码 |
| 混合 B | cuda | CUDA 滤镜 → hwdownload | SW (libx264/265) | GPU 解码/滤镜 → CPU 编码 |
| 回退 | SW | SW 滤镜 | SW | 纯软件 |

滤镜链：`scale_cuda` → `tonemap_cuda` → `overlay_cuda` → `transpose_cuda`

### 5.2 AMD AMF 管线 (3 种)

| 路径 | Decode | Filter | Encode | 说明 |
|------|--------|--------|--------|------|
| **首选** | d3d11va | hwmap → **OpenCL** 滤镜 → hwmap | amf | D3D11↔OpenCL interop |
| 混合 | SW | hwupload → d3d11va → hwmap → **OpenCL** | amf | |
| 回退 | SW | SW 滤镜 | SW / amf | copy-back |

滤镜链：`scale_opencl` → `tonemap_opencl` → `overlay_opencl` → `transpose_opencl`

### 5.3 Intel QSV 管线 (6 种)

**Linux 路径：**

| 路径 | Decode | Filter | Encode | 说明 |
|------|--------|--------|--------|------|
| **首选 (VPP)** | vaapi | tonemap_vaapi (VPP) | qsv | iHD 驱动, Gen9+ |
| 首选 (OCL) | vaapi | hwmap → **OpenCL** 滤镜 | qsv | VAAPI↔OpenCL interop |
| 混合 | SW | hwupload → **OpenCL** | qsv | |

**Windows 路径：**

| 路径 | Decode | Filter | Encode | 说明 |
|------|--------|--------|--------|------|
| **首选** | d3d11va | hwmap → **OpenCL** 滤镜 | qsv | D3D11↔OpenCL interop |
| 混合 | SW | hwupload → **OpenCL** | qsv | |
| 回退 | SW | SW 滤镜 | SW | |

### 5.4 VAAPI 管线 (4 种，按驱动区分)

| 路径 | Decode | Filter | Encode | 适用 |
|------|--------|--------|--------|------|
| **Intel iHD 首选** | vaapi | tonemap_vaapi + **OpenCL** 滤镜 | vaapi | Intel iHD 驱动 |
| **AMD 首选** | vaapi | hwmap → **Vulkan (libplacebo)** → hwmap | vaapi | AMD radeonsi, kernel≥5.1 |
| i965 受限 | vaapi | scale_vaapi + deinterlace_vaapi | vaapi | Intel i965 / AMD 旧驱动 |
| 回退 | SW | SW 滤镜 → hwupload | vaapi | |

关键 interop:
- Intel iHD: `vaapi → hwmap → opencl (滤镜) → hwmap → vaapi`
- AMD radeonsi: `vaapi → hwmap → vulkan (libplacebo) → hwmap → vaapi`

### 5.5 Apple VideoToolbox 管线 (3 种)

| 路径 | Decode | Filter | Encode | 说明 |
|------|--------|--------|--------|------|
| **首选** | videotoolbox | VT 原生滤镜 | videotoolbox | macOS, FFmpeg≥7.0.1 |
| 混合 | SW | hwupload → VT 滤镜 | videotoolbox | |
| 回退 | videotoolbox | SW 滤镜 | SW / videotoolbox | 禁用 HW 滤镜 |

滤镜链：`scale_vt` → `tonemap_videotoolbox` → `overlay_videotoolbox` → `transpose_vt`

### 5.6 Rockchip RKMPP 管线 (3 种)

| 路径 | Decode | Filter | Encode | 说明 |
|------|--------|--------|--------|------|
| **首选** | rkmpp | rkrga + **OpenCL** 滤镜 | rkmpp | AFBC 零拷贝路径 |
| 混合 | SW | rkrga | rkmpp | |
| 回退 | SW | SW 滤镜 | SW | |

### 5.7 V4L2M2M 管线 (1 种)

| 路径 | Decode | Filter | Encode | 说明 |
|------|--------|--------|--------|------|
| 唯一 | v4l2m2m / SW | SW 滤镜 | h264_v4l2m2m | 无 HW 滤镜支持 |

---

## 6. 跨后端 Interop 映射

Jellyfin 通过 FFmpeg 的 `hwmap` / `hwupload` 实现跨硬件后端的零拷贝或低开销数据传输：

| Interop | 方向 | 使用场景 | 机制 |
|---------|------|---------|------|
| **OpenCL ↔ D3D11VA** | 双向 | AMD AMF (Win), Intel QSV (Win) | d3d11-opencl interop |
| **OpenCL ↔ VAAPI** | 双向 | Intel iHD (Linux), AMD (Linux) | vaapi-opencl interop |
| **Vulkan ↔ VAAPI** | 双向 | AMD radeonsi (Linux) | DRM PRIME + format modifier |
| **OpenCL ↔ QSV** | 双向 | Intel 跨设备 tonemap | qsv-opencl interop |
| **OpenCL ↔ RKMPP** | 双向 | Rockchip ARM tonemap | drm_prime-opencl interop |

---

## 7. 回退机制

Jellyfin 的优雅降级链：

```
1. 全 HW 首选管线 (decode+filter+encode 全在 GPU)
   ↓ 失败
2. 跨设备混合管线 (hwmap interop)
   ↓ 失败
3. 混合管线 (HW decode/encode + SW filter)
   ↓ 失败
4. SW decode + HW encode (hwupload)
   ↓ 失败
5. 纯软件 (libx264/libx265/libsvtav1)
```

---

## 8. 关键代码定位

| 功能 | 方法名 | 行号 |
|------|--------|------|
| 滤镜链总调度 | `GetVideoFilterChain()` switch | L6148-6158 |
| 解码器调度 | `GetHardwareVideoDecoder()` | L6317 |
| 编码器选择 | `GetH26xOrAv1Encoder()` | L195 |
| HW 加速类型 | `GetHwaccelType()` | L6478 |
| NVIDIA 滤镜 | `GetNvidiaVidFiltersPrefered()` | L3881 |
| AMD 滤镜 | `GetAmdDx11VidFiltersPrefered()` | L4095 |
| Intel QSV VAAPI | `GetIntelQsvVaapiVidFiltersPrefered()` | L4633 |
| Intel QSV D3D11 | `GetIntelQsvDx11VidFiltersPrefered()` | L4339 |
| VAAPI Intel iHD | `GetIntelVaapiFullVidFiltersPrefered()` | L4966 |
| VAAPI AMD Vulkan | `GetAmdVaapiFullVidFiltersPrefered()` | L5204 |
| Apple VT | `GetAppleVidFiltersPreferred()` | L5679 |
| RKMPP | `GetRkmppVidFiltersPrefered()` | L5874 |
| 纯软件 | `GetSwVidFilterChain()` | L3731 |
