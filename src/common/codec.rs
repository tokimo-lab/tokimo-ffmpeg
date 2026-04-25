//! Codec name normalization.
//!
//! Our probe uses `avcodec_descriptor_get()->name` — the same source as CLI ffprobe.
//! Source and client-reported codec names therefore already use the canonical
//! descriptor names (`"h264"`, `"hevc"`, `"dts"`, etc.) with no aliasing needed.
//!
//! `normalize_video_codec` / `normalize_audio_codec` are lightweight lowercase
//! passes kept for defensive call-site consistency.
//!
//! `normalize_subtitle_codec` does real work: CLI outputs long descriptor names
//! (`"hdmv_pgs_subtitle"`, `"subrip"`, `"dvd_subtitle"`, …) that need to be
//! mapped to the shorter canonical names used throughout the frontend.

/// Normalize a video codec name. Source is CLI ffprobe descriptor name.
/// Just lowercases — no aliasing needed.
pub fn normalize_video_codec(codec: &str) -> String {
    codec.to_lowercase()
}

/// Normalize an audio codec name. Source is CLI ffprobe descriptor name.
/// Just lowercases — no aliasing needed.
pub fn normalize_audio_codec(codec: &str) -> String {
    codec.to_lowercase()
}

/// Normalize a subtitle codec name from CLI ffprobe descriptor name to the
/// shorter canonical form used throughout the app.
///
/// CLI descriptor names → canonical:
///   `"hdmv_pgs_subtitle"` → `"pgs"`
///   `"dvd_subtitle"`      → `"dvdsub"`
///   `"dvb_subtitle"`      → `"dvbsub"`
///   `"subrip"`            → `"srt"`
///   `"ssa"`               → `"ass"`
///   `"webvtt"`            → `"vtt"`
///   `"mov_text"` / `"text"` / `"hdmv_text_subtitle"` → `"srt"`
///   `"eia_608"`           → `"cc"`
///   `"dvb_teletext"`      → `"teletext"`
pub fn normalize_subtitle_codec(codec: &str) -> String {
    let lower = codec.to_lowercase();
    match lower.as_str() {
        "hdmv_pgs_subtitle" => "pgs".to_string(),
        "dvd_subtitle" => "dvdsub".to_string(),
        "dvb_subtitle" => "dvbsub".to_string(),
        "subrip" | "mov_text" | "text" | "hdmv_text_subtitle" => "srt".to_string(),
        "ssa" => "ass".to_string(),
        "webvtt" => "vtt".to_string(),
        "eia_608" => "cc".to_string(),
        "dvb_teletext" => "teletext".to_string(),
        _ => lower,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_video_codec_normalization() {
        assert_eq!(normalize_video_codec("h264"), "h264");
        assert_eq!(normalize_video_codec("hevc"), "hevc");
        assert_eq!(normalize_video_codec("av1"), "av1");
        assert_eq!(normalize_video_codec("vp9"), "vp9");
        assert_eq!(normalize_video_codec("vp8"), "vp8");
        assert_eq!(normalize_video_codec("H264"), "h264");
    }

    #[test]
    fn test_audio_codec_normalization() {
        assert_eq!(normalize_audio_codec("aac"), "aac");
        assert_eq!(normalize_audio_codec("mp3"), "mp3");
        assert_eq!(normalize_audio_codec("ac3"), "ac3");
        assert_eq!(normalize_audio_codec("eac3"), "eac3");
        assert_eq!(normalize_audio_codec("dts"), "dts");
        assert_eq!(normalize_audio_codec("truehd"), "truehd");
        assert_eq!(normalize_audio_codec("mlp"), "mlp");
        assert_eq!(normalize_audio_codec("opus"), "opus");
        assert_eq!(normalize_audio_codec("flac"), "flac");
        assert_eq!(normalize_audio_codec("AAC"), "aac");
    }

    #[test]
    fn test_subtitle_codec_normalization() {
        // CLI descriptor names → canonical
        assert_eq!(normalize_subtitle_codec("hdmv_pgs_subtitle"), "pgs");
        assert_eq!(normalize_subtitle_codec("dvd_subtitle"), "dvdsub");
        assert_eq!(normalize_subtitle_codec("dvb_subtitle"), "dvbsub");
        assert_eq!(normalize_subtitle_codec("subrip"), "srt");
        assert_eq!(normalize_subtitle_codec("ssa"), "ass");
        assert_eq!(normalize_subtitle_codec("webvtt"), "vtt");
        assert_eq!(normalize_subtitle_codec("mov_text"), "srt");
        assert_eq!(normalize_subtitle_codec("text"), "srt");
        assert_eq!(normalize_subtitle_codec("hdmv_text_subtitle"), "srt");
        assert_eq!(normalize_subtitle_codec("eia_608"), "cc");
        assert_eq!(normalize_subtitle_codec("dvb_teletext"), "teletext");

        // Already canonical — pass through
        assert_eq!(normalize_subtitle_codec("srt"), "srt");
        assert_eq!(normalize_subtitle_codec("ass"), "ass");
        assert_eq!(normalize_subtitle_codec("vtt"), "vtt");
        assert_eq!(normalize_subtitle_codec("ttml"), "ttml");
        assert_eq!(normalize_subtitle_codec("microdvd"), "microdvd");
        assert_eq!(normalize_subtitle_codec("xsub"), "xsub");
    }
}
