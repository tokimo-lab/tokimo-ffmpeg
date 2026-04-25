mod dts;
pub mod encode;
pub mod filter;
pub mod hw;
mod pass;
pub mod pipeline;
pub mod scheduler;
mod session;
pub mod types;
mod vfs_io;

use crate::error::{Error, Result};
use rsmpeg::avformat::AVFormatContextInput;
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};
use std::time::Instant;

use hw::{HwAccel, HwPipeline};
use session::setup_session;

// Re-export all public types from types module
pub use types::{
    CancellationToken, DirectInput, HlsOptions, HlsSegmentType, PauseToken, SeekCommand, TranscodeOptions,
    cancellation_token, pause_token,
};
pub use vfs_io::READAHEAD_HLS;
pub(crate) use vfs_io::open_direct_input;

// Jellyfin approach: no H.264 NAL manipulation in copy mode.
// I-slices (open-GOP) are passed through as-is. HLS.js correctly detects
// I-slices as keyframes by parsing the slice header (slice_type 2/4/7/9).

/// Run a single transcode pass using an already-opened input context.
/// Called once for the initial pass and again for each seek-restart.
fn run_pass(
    ifmt_ctx: &mut AVFormatContextInput,
    opts: &TranscodeOptions,
    decode_accel: &Option<HwAccel>,
    encode_accel: &Option<HwAccel>,
    pipeline_cfg: &HwPipeline,
) -> Result<()> {
    pass::run_pass(ifmt_ctx, opts, decode_accel, encode_accel, pipeline_cfg)
}

// ── Public entry points ─────────────────────────────────────────────────────

/// One-shot transcode (non-HLS or single-pass use).
pub fn transcode(opts: &TranscodeOptions) -> Result<()> {
    let t_total_start = Instant::now();
    let _output_path = opts
        .output
        .to_str()
        .ok_or_else(|| Error::Other("Output path contains invalid UTF-8".into()))?;

    let mut s = setup_session(opts)?;
    run_pass(&mut s.ifmt_ctx, opts, &s.decode_accel, &s.encode_accel, &s.pipeline_cfg)?;

    let total_ms = t_total_start.elapsed().as_secs_f64() * 1000.0;
    let _work_ms = total_ms - s.hw_init_ms;
    tracing::info!(
        "Transcoding completed (total {:.0}ms, hw_init {:.0}ms)",
        total_ms,
        s.hw_init_ms
    );
    Ok(())
}

/// Persistent HLS transcode session: opens input ONCE, then loops on seek commands.
///
/// On each `SeekCommand::Seek`, seeks within the already-opened demuxer and runs
/// a new transcode pass — skipping the ~600ms input open/probe overhead.
/// On `SeekCommand::Stop` (or channel close), exits cleanly.
///
/// `on_pass_finish(completed)` is called after each pass:
///   - `true` = pass reached EOF or finished normally
///   - `false` = pass was cancelled (seek-restart or stop)
pub fn transcode_session<F>(
    opts: &TranscodeOptions,
    seek_rx: mpsc::Receiver<SeekCommand>,
    on_pass_finish: F,
) -> Result<()>
where
    F: Fn(bool) + Send + Sync,
{
    let _output_path = opts
        .output
        .to_str()
        .ok_or_else(|| Error::Other("Output path contains invalid UTF-8".into()))?
        .to_string();

    let mut s = setup_session(opts)?;

    // Initial pass
    let t_pass = Instant::now();
    run_pass(&mut s.ifmt_ctx, opts, &s.decode_accel, &s.encode_accel, &s.pipeline_cfg)?;
    let initial_cancelled = opts.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed));
    on_pass_finish(!initial_cancelled);
    tracing::info!(
        "Transcoding completed (initial pass {:.0}ms, hw_init {:.0}ms)",
        t_pass.elapsed().as_secs_f64() * 1000.0,
        s.hw_init_ms
    );

    // Seek-restart loop — demuxer stays open, only output pipeline is recreated
    for cmd in seek_rx {
        match cmd {
            SeekCommand::Seek {
                seek_secs,
                start_segment,
                cancel,
                pause,
            } => {
                let t_cmd = Instant::now();
                let pass_opts = TranscodeOptions {
                    seek: Some(seek_secs),
                    cancel: Some(cancel.clone()),
                    pause: Some(pause),
                    hls: opts.hls.clone().map(|mut h| {
                        h.start_number = start_segment;
                        h
                    }),
                    ..opts.clone()
                };

                tracing::debug!(
                    "[transcode] Seek command received (queued {:.0}ms)",
                    t_cmd.elapsed().as_secs_f64() * 1000.0
                );

                let t_pass = Instant::now();
                match run_pass(
                    &mut s.ifmt_ctx,
                    &pass_opts,
                    &s.decode_accel,
                    &s.encode_accel,
                    &s.pipeline_cfg,
                ) {
                    Ok(()) => {
                        let cancelled = cancel.load(Ordering::Relaxed);
                        on_pass_finish(!cancelled);
                        tracing::info!(
                            "Transcoding completed (pass {:.0}ms)",
                            t_pass.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                    Err(e) => {
                        if cancel.load(Ordering::Relaxed) {
                            on_pass_finish(false);
                            tracing::info!(
                                "Transcode pass cancelled ({:.0}ms)",
                                t_pass.elapsed().as_secs_f64() * 1000.0
                            );
                        } else {
                            return Err(e);
                        }
                    }
                }
            }
            SeekCommand::Stop => break,
        }
    }

    Ok(())
}

/// Probe a `DirectInput` (VFS-backed AVIO source) and return `MediaInfo`.
///
/// Same as `probe_file` but opens via a custom AVIO context backed by the
/// provided `DirectInput` callback instead of a filesystem/HTTP URL.
/// Used for remote ISO files where we expose only the inner M2TS stream.
pub fn probe_direct(input: Arc<DirectInput>) -> crate::error::Result<crate::media::probe::MediaInfo> {
    let mut probe_opts =
        Some(rsmpeg::avutil::AVDictionary::new(c"probesize", c"50000000", 0).set(c"analyzeduration", c"50000000", 0));
    let input_ctx = vfs_io::open_direct_input(input, &mut probe_opts)?;
    Ok(crate::media::probe::probe_format_ctx(input_ctx, "direct", "0"))
}
