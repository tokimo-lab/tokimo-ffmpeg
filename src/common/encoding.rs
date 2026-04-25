//! Shared encoding utilities — audio/video bitrate calculation, channel normalization,
//! and codec compatibility checks.
//!
//! Extracted from `rust-hls/src/ffmpeg.rs` and `rust-hls/src/types.rs` so that
//! any crate in the workspace can reuse these without depending on `rust-hls`.

use super::codec::normalize_audio_codec;

// ═══════════════════════════════════════════════════════════════════════════════
//  Audio codec compatibility
// ═══════════════════════════════════════════════════════════════════════════════

/// Audio codecs that browsers cannot play natively (canonical names only).
/// Raw ffprobe values are first normalized via [`crate::normalize_audio_codec`]
/// so aliases like `dca` → `dts`, `mlp` → `truehd`, `ec3` → `eac3` are handled.
const UNSUPPORTED_AUDIO_CODECS: &[&str] = &[
    "ac3",
    "eac3",
    "dts",
    "truehd",
    "pcm_s16le",
    "pcm_s24le",
    "pcm_s32le",
    "pcm_bluray",
    "pcm_dvd",
];

/// Quick check: does this audio codec need transcoding for browser playback?
///
/// Uses a **blacklist** of known-unsupported codecs. Anything not in the list
/// passes through (conservative — avoids unnecessary transcoding).
///
/// For a richer client-profile-aware decision, see
/// `rust-server::services::transcode_decision::audio_transcode_reason()`.
pub fn needs_audio_transcode(codec: &str) -> bool {
    let normalized = normalize_audio_codec(codec);
    UNSUPPORTED_AUDIO_CODECS.contains(&normalized.as_str())
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Audio channel / bitrate helpers
// ═══════════════════════════════════════════════════════════════════════════════

/// Maximum AAC output channels (5.1).
pub const MAX_AAC_CHANNELS: u32 = 6;

/// AAC bitrate per channel (kbps).
pub const BITRATE_PER_CHANNEL: u32 = 128;

/// Maximum total AAC bitrate (kbps) for surround (≥6ch).
pub const MAX_SURROUND_BITRATE: u32 = 640;

/// Normalize audio channels to browser/HLS-compatible values.
///
/// Mapping rules:
/// - 0, 1 → Mono (1)
/// - 2 → Stereo (2)
/// - 3, 4 → Downmix to stereo (2)
/// - 5 → Upmix to 5.1 (6) — adds LFE
/// - 6 → 5.1 passthrough (6)
/// - 7 → 7.1 (8)
/// - 8+ → Passthrough
pub fn normalize_output_channels(channels: u32) -> u32 {
    match channels {
        0 | 1 => 1,
        2 => 2,
        3..=5 => {
            if channels == 5 {
                6
            } else {
                2
            }
        }
        6 => 6,
        7 => 8,
        _ => channels,
    }
}

/// Calculate the output audio bitrate (kbps) for the given raw channel count.
///
/// Normalizes channels first, then applies per-channel rate capped at
/// [`MAX_SURROUND_BITRATE`] for surround layouts.
///
/// Returns `(bitrate_kbps, normalized_channels)`.
pub fn calculate_audio_output(channels: u32) -> (u32, u32) {
    let out_channels = normalize_output_channels(channels).min(MAX_AAC_CHANNELS);
    let bitrate_kbps = if out_channels >= 6 {
        MAX_SURROUND_BITRATE
    } else {
        out_channels * BITRATE_PER_CHANNEL
    };
    (bitrate_kbps, out_channels)
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Video bitrate scaling (Jellyfin ScaleBitrate)
// ═══════════════════════════════════════════════════════════════════════════════

/// Default output video bitrate when source bitrate is unknown (kbps).
pub const DEFAULT_VIDEO_BITRATE_KBPS: u64 = 8000;

/// Calculate output video bitrate using Jellyfin's `ScaleBitrate` logic.
///
/// Mirrors `EncodingHelper.GetVideoBitrateParamValue` + `ScaleBitrate`:
/// 1. Get codec efficiency scale factors (HEVC/VP9=0.6, AV1=0.5, H264=1.0)
/// 2. Scale = max(outputFactor / inputFactor, 1) — never reduce bitrate
/// 3. Boost low bitrates: ≤500k→4x, ≤1M→3x, ≤2M→2.5x, ≤3M→2x
/// 4. Cap: don't scale beyond 30Mbps (diminishing returns with fast presets)
pub fn calculate_output_video_bitrate(source_bitrate_bps: Option<u64>, input_codec: &str, output_codec: &str) -> u64 {
    let source_bps = match source_bitrate_bps {
        Some(b) if b > 0 => b,
        _ => return DEFAULT_VIDEO_BITRATE_KBPS,
    };

    fn codec_scale_factor(codec: &str) -> f64 {
        let normalized = crate::normalize_video_codec(codec);
        match normalized.as_str() {
            "hevc" | "vp9" => 0.6,
            "av1" => 0.5,
            _ => 1.0,
        }
    }

    let input_factor = codec_scale_factor(input_codec);
    let output_factor = codec_scale_factor(output_codec);
    let mut scale = (output_factor / input_factor).max(1.0);

    // Jellyfin: boost low bitrates for better quality
    if source_bps <= 500_000 {
        scale = scale.max(4.0);
    } else if source_bps <= 1_000_000 {
        scale = scale.max(3.0);
    } else if source_bps <= 2_000_000 {
        scale = scale.max(2.5);
    } else if source_bps <= 3_000_000 {
        scale = scale.max(2.0);
    } else if source_bps >= 30_000_000 {
        // Jellyfin: "Don't scale beyond 30Mbps, hardly visually noticeable
        // for most codecs with our prefer-speed encoding"
        scale = 1.0;
    }

    let result_bps = (scale * source_bps as f64) as u64;
    // Return in kbps, cap at int_max/2 to satisfy bufsize=bitrate*2
    (result_bps / 1000).min(i32::MAX as u64 / 2)
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_needs_audio_transcode() {
        // Canonical names
        assert!(needs_audio_transcode("ac3"));
        assert!(needs_audio_transcode("truehd"));
        assert!(needs_audio_transcode("DTS"));
        assert!(needs_audio_transcode("pcm_s16le"));
        assert!(needs_audio_transcode("pcm_s24le"));
        assert!(needs_audio_transcode("pcm_bluray"));
        assert!(!needs_audio_transcode("aac"));
        assert!(!needs_audio_transcode("mp3"));
        assert!(!needs_audio_transcode("opus"));

        // ffprobe external decoder names — must NOT trigger transcode
        assert!(!needs_audio_transcode("libfdk_aac")); // → aac
        assert!(!needs_audio_transcode("libopus")); // → opus
        assert!(!needs_audio_transcode("libvorbis")); // → vorbis
        assert!(!needs_audio_transcode("libmp3lame")); // → mp3
    }

    #[test]
    fn test_normalize_output_channels() {
        assert_eq!(normalize_output_channels(0), 1);
        assert_eq!(normalize_output_channels(1), 1);
        assert_eq!(normalize_output_channels(2), 2);
        assert_eq!(normalize_output_channels(3), 2);
        assert_eq!(normalize_output_channels(4), 2);
        assert_eq!(normalize_output_channels(5), 6);
        assert_eq!(normalize_output_channels(6), 6);
        assert_eq!(normalize_output_channels(7), 8);
        assert_eq!(normalize_output_channels(8), 8);
    }

    #[test]
    fn test_calculate_audio_output() {
        // Mono
        let (br, ch) = calculate_audio_output(1);
        assert_eq!(ch, 1);
        assert_eq!(br, 128);

        // Stereo
        let (br, ch) = calculate_audio_output(2);
        assert_eq!(ch, 2);
        assert_eq!(br, 256);

        // 5.1
        let (br, ch) = calculate_audio_output(6);
        assert_eq!(ch, 6);
        assert_eq!(br, MAX_SURROUND_BITRATE);

        // 5ch upmixed to 5.1
        let (br, ch) = calculate_audio_output(5);
        assert_eq!(ch, 6);
        assert_eq!(br, MAX_SURROUND_BITRATE);
    }

    #[test]
    fn test_video_bitrate_scaling() {
        // No source → default
        assert_eq!(
            calculate_output_video_bitrate(None, "hevc", "h264"),
            DEFAULT_VIDEO_BITRATE_KBPS
        );
        assert_eq!(
            calculate_output_video_bitrate(Some(0), "hevc", "h264"),
            DEFAULT_VIDEO_BITRATE_KBPS
        );

        // HEVC → H264: factor = 1.0/0.6 ≈ 1.67
        let result = calculate_output_video_bitrate(Some(10_000_000), "hevc", "h264");
        assert!(result > 10_000); // scaled up

        // H264 → H264: factor = 1.0
        let result = calculate_output_video_bitrate(Some(10_000_000), "h264", "h264");
        assert_eq!(result, 10_000);

        // Very high bitrate (≥30Mbps) → no scaling
        let result = calculate_output_video_bitrate(Some(40_000_000), "hevc", "h264");
        assert_eq!(result, 40_000);
    }
}
