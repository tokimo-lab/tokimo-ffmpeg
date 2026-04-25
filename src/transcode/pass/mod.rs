mod analyze;
mod config;
mod demux;
mod encode;
mod flush;
mod gpu;
mod output;
mod targets;

use crate::error::Result;
use rsmpeg::avformat::{AVFormatContextInput, AVFormatContextOutput};
use std::time::Instant;

use super::hw::{HwAccel, HwPipeline};
use super::types::TranscodeOptions;

/// Run a single transcode pass.
///
/// Phases are chained via phase-output structs.  Each phase produces a fully
/// initialized value that subsequent phases consume.  Variable scoping enforces
/// ordering at compile time: a phase cannot accidentally use outputs from a
/// later phase.
pub(crate) fn run_pass(
    ifmt_ctx: &mut AVFormatContextInput,
    opts: &TranscodeOptions,
    decode_accel: &Option<HwAccel>,
    encode_accel: &Option<HwAccel>,
    pipeline_cfg: &HwPipeline,
) -> Result<()> {
    let t_pass = Instant::now();

    // Phase 1: Derive immutable configuration
    let cfg = config::PassConfig::new(ifmt_ctx, opts, decode_accel, encode_accel)?;

    // Phase 2: Seek input (side effect on ifmt_ctx)
    analyze::seek(ifmt_ctx, opts, &cfg)?;

    // Phase 3: Analyze streams → stream mapping + decoder contexts
    let (analysis, mut dec_ctxs) = analyze::analyze_streams(ifmt_ctx, opts, &cfg, decode_accel)?;

    // Phase 4: Determine encode targets per stream
    let targets = targets::determine_targets(&analysis, &dec_ctxs, &cfg, ifmt_ctx, opts)?;

    // Phase 5: Open output context
    let output_path = opts.output.to_str().unwrap_or("output");
    let c_output = std::ffi::CString::new(output_path)?;
    let mut ofmt_ctx = AVFormatContextOutput::create(&c_output)?;

    // Phase 6: Build filter/encoder pipeline (adds streams to ofmt_ctx)
    let mut pipeline = encode::create_pipeline(
        &analysis,
        &mut dec_ctxs,
        &targets,
        &cfg,
        opts,
        ifmt_ctx,
        &mut ofmt_ctx,
        decode_accel,
        encode_accel,
        pipeline_cfg,
    )?;

    // Phase 7: Configure muxer options + write header
    output::configure_output(&mut ofmt_ctx, opts, &cfg)?;

    tracing::info!(
        "[transcode] Pipeline ready in {:.0}ms",
        t_pass.elapsed().as_secs_f64() * 1000.0
    );

    // Phase 8: Main demux → dispatch → mux loop
    demux::run_demux_loop(
        ifmt_ctx,
        opts,
        &cfg,
        &analysis,
        &mut dec_ctxs,
        &targets,
        &mut pipeline,
        &mut ofmt_ctx,
        decode_accel,
    )?;

    // Phase 9: Flush remaining streams + write trailer + cleanup
    flush::flush_and_finalize(
        &cfg,
        &analysis,
        &mut dec_ctxs,
        &targets,
        &mut pipeline,
        &mut ofmt_ctx,
        decode_accel,
    )?;

    tracing::info!(
        "[transcode] Pass completed in {:.0}ms",
        t_pass.elapsed().as_secs_f64() * 1000.0
    );

    Ok(())
}
