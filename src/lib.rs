// tokimo-ffmpeg 是 libav* 的 FFI 绑定层，unsafe 调用是预期行为
// FFI 层中的 println!/eprintln! 是 libav 回调的标准输出方式，unwrap 在 FFI 边界是预期的
#![allow(unsafe_code, clippy::print_stdout, clippy::print_stderr, clippy::unwrap_in_result)]

pub mod common;
pub mod error;
pub mod media;
pub mod transcode;

/// Shorthand macro equivalent to `return Err(Error::Other(format!(...)))`.
#[macro_export]
macro_rules! bail {
    ($($arg:tt)*) => {
        return Err($crate::error::Error::Other(format!($($arg)*)))
    };
}

pub use error::ResultExt;

// ── Media processing re-exports ──────────────────────────────────────
pub use media::audio::{self, AudioConvertOptions, convert_audio, convert_audio_file};
pub use media::image::{self, ImageDecodeOptions, ImageFormat, decode_image, decode_image_from_bytes};
pub use media::probe::{
    self, AudioFields, ChapterInfo, FormatInfo, MediaInfo, SideDataInfo, StreamInfo, VideoFields, probe_file,
};
pub use media::remux::{self, RemuxOptions, extract_audio as remux_extract_audio, merge_av, remux};
pub use media::screenshot::{self, VideoScreenshotOptions, capture_video_screenshot, capture_video_screenshot_direct};

// ── Common / codec knowledge re-exports ──────────────────────────────
pub use common::capabilities::{
    self, CodecInfo, CodecType, FilterInfo, FormatDesc, HwCapabilities, amf_decode_supported, best_aac_encoder,
    best_encoder_for_audio_codec, detect_capabilities, ffmpeg_version, ffmpeg_version_full, get_cuvid_decoder,
    has_decoder, has_encoder, has_filter, list_codecs, list_demuxers, list_filters, list_hwaccels, list_muxers,
    list_protocols, probe_cuvid_hw_codecs, qsv_decode_supported, vaapi_decode_supported, videotoolbox_decode_supported,
};
pub use common::codec::{normalize_audio_codec, normalize_subtitle_codec, normalize_video_codec};
pub use common::encoding::{
    self, BITRATE_PER_CHANNEL, DEFAULT_VIDEO_BITRATE_KBPS, MAX_AAC_CHANNELS, MAX_SURROUND_BITRATE,
    calculate_audio_output, calculate_output_video_bitrate, needs_audio_transcode, normalize_output_channels,
};

// ── Transcode engine re-exports ──────────────────────────────────────
pub use rsmpeg::avutil::AVHWDeviceContext;
/// Synchronous random-access reader: `(offset, max_bytes) -> bytes`.
///
/// Structurally identical to `tokimo_vfs_core::ReadAt` — both crates define
/// `Arc<dyn Fn(u64, usize) -> io::Result<Vec<u8>> + Send + Sync>` which is the
/// same Rust type, so values produced by `tokimo_vfs_core::sync::make_sync_reader`
/// can be passed directly into this crate's APIs.
pub type ReadAt = std::sync::Arc<dyn Fn(u64, usize) -> std::io::Result<Vec<u8>> + Send + Sync>;
pub use transcode::hw::{
    FallbackLevel, FilterBackend, HwAccel, HwPipeline, HwType, build_pipeline, infer_hw_from_codec, parse_hw_type,
    resolve_pipeline, resolve_pipeline_with_fallback,
};
pub use transcode::{
    CancellationToken, DirectInput, HlsOptions, HlsSegmentType, PauseToken, READAHEAD_HLS, SeekCommand,
    TranscodeOptions, cancellation_token, pause_token, probe_direct, transcode, transcode_session,
};
