# tokimo-ffmpeg

高性能媒体转码库及 CLI 工具，基于 Rust FFI 绑定到打过补丁的 FFmpeg，实现了可组合的三阶段硬件管线——解码、滤镜、编码，每个阶段独立配置，自动处理跨设备互操作。

[English](README.md)

## 为什么用 FFI 而不是子进程

| CLI 子进程的问题 | FFI 的解决方案 |
|---|---|
| 每次 seek 启动进程（~30ms fork+exec） | 零开销，同进程调用 |
| 每次 seek 重初始化 CUDA 驱动（~200ms） | 持久 GPU 上下文（热态 ~60ms） |
| 无背压控制（SIGSTOP/SIGCONT hack） | 有界信道帧级流控 |
| 解析 stderr 获取错误信息 | 结构化 AVERROR 错误码 |
| 轮询文件系统获取进度 | 帧级 PTS 回调 |
| 单体式配置 | 可组合三阶段管线 API |

**结果：** 10 次 seek 播放模拟中，比 CLI FFmpeg 快 **29%**。

## 库用法

```rust
use ffmpeg_tool::{
    transcode, TranscodeOptions,
    build_pipeline, resolve_pipeline, HwType, FilterBackend, FallbackLevel,
    probe_file, MediaInfo,
};

// 探测文件信息
let info = probe_file(Path::new("input.mkv"))?;
println!("时长: {}s, 流数量: {}", info.format.duration, info.format.nb_streams);

// 显式构建硬件管线
let pipeline = build_pipeline(
    Some(HwType::Cuda),       // 解码：NVIDIA NVDEC
    FilterBackend::Native,     // 滤镜：scale_cuda / tonemap_cuda
    Some(HwType::Cuda),       // 编码：NVENC
    "hevc_nvenc",
);
assert_eq!(pipeline.fallback, FallbackLevel::FullHw);

// 从编码器名自动推断（最常用）
let pipeline = resolve_pipeline("h264_nvenc", None, None);
// → Cuda → Native → Cuda(h264_nvenc) [全硬件]

// 跨设备管线（VAAPI 解码 → OpenCL 滤镜 → QSV 编码）
let pipeline = build_pipeline(
    Some(HwType::Vaapi),
    FilterBackend::OpenCL,
    Some(HwType::Qsv),
    "hevc_qsv",
);
assert_eq!(pipeline.fallback, FallbackLevel::CrossDevice);

// 执行转码
let opts = TranscodeOptions {
    input: "input.mkv".into(),
    output: "output.mp4".into(),
    video_codec: "hevc_nvenc".into(),
    audio_codec: "aac".into(),
    preset: "p1".into(),
    bitrate: Some("8000k".into()),
    seek: Some(3600.0),
    duration: Some(30.0),
    ..Default::default()
};
transcode(&opts)?;
```

## CLI（测试工具）

```bash
export LD_LIBRARY_PATH=$PWD/install/lib:$PWD/install/deps

# GPU 转码（后端从编码器名自动推断）
ffmpeg-tool transcode input.mkv output.mp4 --video-codec hevc_nvenc --audio-codec aac

# HDR → SDR 色调映射
ffmpeg-tool transcode input.mkv output.mp4 \
  --video-codec h264_nvenc --audio-codec aac --preset p1 --bitrate 8000k \
  --video-filter "setparams=...,tonemap_cuda=format=yuv420p:tonemap=bt2390"

# 跨设备管线
ffmpeg-tool transcode input.mkv output.mp4 \
  --video-codec hevc_qsv --decode vaapi --filter-backend opencl

# 纯软件编码（不需要 GPU）
ffmpeg-tool transcode input.mkv output.mp4 --video-codec libx264 --crf 23

# 10 次 seek 基准测试
ffmpeg-tool bench-seek input.mkv \
  --seeks "0,720,1440,2160,2880,3600,4320,5040,5760,6480" \
  --video-codec h264_nvenc --duration 3 --bitrate 8000k
```

## 三阶段可组合管线

每个阶段独立配置：

```
┌──────────┐     ┌──────────┐     ┌──────────┐
│  解码     │ ──→ │  滤镜    │ ──→ │  编码    │
│  --decode│     │--filter- │     │--video-  │
│  (cuda)  │     │  backend │     │  codec   │
│          │     │(opencl)  │     │(hevc_qsv)│
└──────────┘     └──────────┘     └──────────┘
```

跨设备互操作通过 FFmpeg `hwmap` 实现：
- **OpenCL 桥接**：`vaapi → hwmap=derive_device=opencl → [滤镜] → hwmap → vaapi`
- **Vulkan 桥接**：`vaapi → hwmap=derive_device=vulkan → [libplacebo] → hwmap → vaapi`

### 五级回退链

```
1. 全硬件      同设备解码 + 原生滤镜 + 编码（零拷贝）
   ↓ 失败
2. 跨设备      VAAPI 解码 → hwmap(OpenCL) → QSV 编码
   ↓ 失败
3. 混合 A      软件解码 → hwupload → 硬件滤镜 + 硬件编码
   ↓ 失败
4. 混合 B      硬件解码 → hwdownload → 软件滤镜 + 硬件编码
   ↓ 失败
5. 纯软件      libx264 / libx265 / libsvtav1
```

## 性能基准

**测试文件：** 速度与激情6 — 18GB 4K HEVC HDR (BT.2020/PQ) → H.264 SDR (tonemap_cuda + h264_nvenc)  
**GPU：** NVIDIA RTX 4080 16GB，WSL2

### 10 次 seek 播放模拟（每次 3s，含 HDR 色调映射）

| Seek 位置 | CLI FFmpeg | Rust FFI | 差值 |
|---|---|---|---|
| 0s | 854 ms | 798 ms | −7% |
| 720s（12分） | 852 ms | 559 ms | −34% |
| 1440s（24分） | 991 ms | 683 ms | −31% |
| 2160s（36分） | 912 ms | 589 ms | −35% |
| 2880s（48分） | 996 ms | 698 ms | −30% |
| 3600s（1小时） | 904 ms | 573 ms | −37% |
| 4320s（1时12分） | 958 ms | 632 ms | −34% |
| 5040s（1时24分） | 1249 ms | 991 ms | −21% |
| 5760s（1时36分） | 1203 ms | 893 ms | −26% |
| 6480s（1时48分） | 905 ms | 561 ms | −38% |
| **平均** | **982 ms** | **698 ms** | **−29%** |

### 直接转码对比（10s 片段，无色调映射）

| 编码器 | CLI FFmpeg | Rust FFI | 差值 |
|---|---|---|---|
| HEVC NVENC | 1894 ms | 1728 ms | −8.8% |
| H264 NVENC | 1874 ms | 1723 ms | −8.1% |
| AV1 NVENC | 2493 ms | 2379 ms | −4.6% |

## 硬件支持

### 7 种硬件后端

| 后端 | 类型 | 平台 | 解码 | 滤镜 | 编码 |
|---|---|---|---|---|---|
| **NVIDIA CUDA** | `HwType::Cuda` | Linux/Windows | ✅ NVDEC | ✅ scale/tonemap/overlay_cuda | ✅ NVENC |
| **AMD AMF** | `HwType::Amf` | Windows | ✅ D3D11VA | ✅ OpenCL 桥接 | ✅ AMF |
| **Intel QSV** | `HwType::Qsv` | Linux/Windows | ✅ QSV | ✅ VPP + OpenCL 桥接 | ✅ QSV |
| **VAAPI** | `HwType::Vaapi` | Linux | ✅ VAAPI | ✅ 原生 + OpenCL/Vulkan 桥接 | ✅ VAAPI |
| **VideoToolbox** | `HwType::Videotoolbox` | macOS | ✅ VT | ✅ scale/tonemap_vt | ✅ VT |
| **RKMPP** | `HwType::Rkmpp` | Linux ARM | ✅ RKMPP | ✅ RKRGA + OpenCL 桥接 | ✅ RKMPP |
| **V4L2M2M** | `HwType::V4l2m2m` | Linux | ✅ V4L2 | 仅软件 | ✅ V4L2 |

### 编码器矩阵

| 编码格式 | NVIDIA | AMD AMF | Intel QSV | VAAPI | VideoToolbox | RKMPP | V4L2M2M |
|---|---|---|---|---|---|---|---|
| H.264 | h264_nvenc | h264_amf | h264_qsv | h264_vaapi | h264_videotoolbox | h264_rkmpp | h264_v4l2m2m |
| HEVC | hevc_nvenc | hevc_amf | hevc_qsv | hevc_vaapi | hevc_videotoolbox | hevc_rkmpp | — |
| AV1 | av1_nvenc | av1_amf | av1_qsv | av1_vaapi | — | — | — |

### 滤镜矩阵

| 滤镜 | CUDA | OpenCL | Vulkan | VAAPI | QSV VPP | VideoToolbox | RKRGA |
|---|---|---|---|---|---|---|---|
| **缩放** | scale_cuda | scale_opencl | scale_vulkan | scale_vaapi | scale_qsv | scale_vt | scale_rkrga |
| **色调映射** | tonemap_cuda | tonemap_opencl | libplacebo | tonemap_vaapi | vpp_qsv | tonemap_videotoolbox | — |
| **去隔行** | yadif_cuda | yadif_opencl | — | deinterlace_vaapi | deinterlace_qsv | yadif_videotoolbox | — |
| **叠加** | overlay_cuda | overlay_opencl | overlay_vulkan | overlay_vaapi | overlay_qsv | overlay_videotoolbox | overlay_rkrga |
| **旋转** | transpose_cuda | transpose_opencl | transpose_vulkan | transpose_vaapi | — | transpose_vt | — |

### 跨设备互操作（5 种）

| 互操作类型 | 方向 | 用途 |
|---|---|---|
| OpenCL ↔ D3D11VA | 双向 | AMD AMF（Windows）、Intel QSV（Windows）|
| OpenCL ↔ VAAPI | 双向 | Intel iHD（Linux）、AMD（Linux）|
| Vulkan ↔ VAAPI | 双向 | AMD radeonsi + libplacebo |
| OpenCL ↔ QSV | 双向 | Intel 跨设备色调映射 |
| OpenCL ↔ RKMPP | 双向 | Rockchip ARM 色调映射 |

## 代码结构

```
src/
├── lib.rs                   # 库入口：导出 transcode、probe、硬件类型
├── main.rs                  # CLI 测试工具（clap）
├── probe.rs        (615L)   # 媒体探测（JSON 输出）
└── transcode/
    ├── mod.rs      (989L)   # 调度器：打开 → 管线 → 复用
    ├── hw.rs       (499L)   # 三阶段管线：HwType、FilterBackend、HwPipeline
    ├── filter.rs   (363L)   # 滤镜图：统一 + 跨设备 hwmap
    ├── encode.rs   (167L)   # 编码器设置、选项、flush
    └── pipeline.rs (276L)   # 多线程管线（解码 / 滤镜+编码 / 音频）
```

### 管线线程模型

```
主线程：    解复用 ─────────────────────────── 复用
              │                                  ▲
              ▼                                  │
解码线程：  硬件解码 → 发送帧                    │
                          │                      │
                          ▼                      │
滤镜+编码： [hwmap] → 滤镜 → [hwmap] → 编码 ───┘
音频线程：  解码 → aformat → aac 编码 ──────────┘
```

## 构建

```bash
# Docker 构建（推荐，处理所有 80+ 个依赖）
make docker && make docker-deps && make rust-build

# 设置运行时库路径
export LD_LIBRARY_PATH=$PWD/install/lib:$PWD/install/deps
```

详见 [docs/build-guide.md](docs/build-guide.md)。

## 文档

| 文档 | 说明 |
|---|---|
| [docs/architecture.md](docs/architecture.md) | 模块结构、管线设计、内存模型 |
| [docs/benchmarks.md](docs/benchmarks.md) | 10 次 seek 基准、编码器对比 |
| [docs/build-guide.md](docs/build-guide.md) | 前置条件、Docker/本地构建、故障排查 |
| [docs/cli-usage.md](docs/cli-usage.md) | 所有子命令、选项、示例 |
| [docs/test-scripts.md](docs/test-scripts.md) | 验证测试、基准脚本、CLI 对比 |
| [docs/jellyfin-hw-pipelines.md](docs/jellyfin-hw-pipelines.md) | 硬件管线分析：25+ 条管线路径 |
| [docs/tokimo-hw-support.md](docs/tokimo-hw-support.md) | 硬件后端覆盖情况表 |

## 许可证

依赖带有非自由编解码器（fdk-aac）的 GPL 版 FFmpeg，编译后的 FFmpeg 库为 GPL v3。
