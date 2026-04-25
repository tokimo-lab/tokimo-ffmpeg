use crate::common::capabilities::{
    amf_decode_supported, detect_capabilities, get_cuvid_decoder, qsv_decode_supported, vaapi_decode_supported,
    videotoolbox_decode_supported,
};
use crate::error::{Error, Result};
use crate::media::image::{ImageDecodeOptions, ImageFormat, encode_image_frame};
use crate::transcode::hw::{HwAccel, HwType};
use crate::transcode::{DirectInput, open_direct_input};
use rsmpeg::{
    avcodec::{AVCodec, AVCodecContext},
    avformat::AVFormatContextInput,
    avutil::{AVDictionary, AVFrame},
    error::RsmpegError,
    ffi,
    swscale::SwsContext,
};
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::warn;

/// Options for one-shot video screenshot extraction.
#[derive(Debug, Clone)]
pub struct VideoScreenshotOptions {
    /// Target timestamp in seconds.
    pub offset_secs: f64,
    /// Optional target width. When omitted, keeps the decoded width.
    pub width: Option<u32>,
    /// Optional target height. When omitted, preserves aspect ratio when width is set.
    pub height: Option<u32>,
    /// Output image format.
    pub format: ImageFormat,
    /// Output quality.
    ///
    /// - JPEG: 1-31 (`2` is high quality)
    /// - WebP: 0-100 (`80` is a good default)
    /// - PNG: ignored
    pub quality: u8,
    /// Try hardware decode first and fall back to software when it fails.
    pub prefer_hardware: bool,
}

impl Default for VideoScreenshotOptions {
    fn default() -> Self {
        Self {
            offset_secs: 0.0,
            width: None,
            height: None,
            format: ImageFormat::Jpeg,
            quality: 2,
            prefer_hardware: true,
        }
    }
}

#[derive(Clone)]
enum ScreenshotInput {
    File(PathBuf),
    Direct(Arc<DirectInput>),
}

#[derive(Debug)]
struct SelectedVideoStream {
    index: usize,
    codec_id: ffi::AVCodecID,
    codec_name: String,
    time_base: ffi::AVRational,
}

/// Capture a single screenshot from a local video file.
pub fn capture_video_screenshot(input_path: &Path, opts: &VideoScreenshotOptions) -> Result<Vec<u8>> {
    capture_with_fallback(ScreenshotInput::File(input_path.to_path_buf()), opts)
}

/// Capture a single screenshot from a `DirectInput` AVIO source.
pub fn capture_video_screenshot_direct(input: Arc<DirectInput>, opts: &VideoScreenshotOptions) -> Result<Vec<u8>> {
    capture_with_fallback(ScreenshotInput::Direct(input), opts)
}

fn capture_with_fallback(source: ScreenshotInput, opts: &VideoScreenshotOptions) -> Result<Vec<u8>> {
    let (first_attempt, used_hw) = capture_once(source.clone(), opts, opts.prefer_hardware);
    match first_attempt {
        Ok(bytes) => Ok(bytes),
        Err(err) if used_hw => {
            warn!("[screenshot] hardware decode failed, retrying in software: {err}");
            match capture_once(source, opts, false).0 {
                Ok(bytes) => Ok(bytes),
                Err(sw_err) => Err(Error::Other(format!(
                    "hardware screenshot failed: {err}; software fallback failed: {sw_err}"
                ))),
            }
        }
        Err(err) => Err(err),
    }
}

fn capture_once(source: ScreenshotInput, opts: &VideoScreenshotOptions, allow_hw: bool) -> (Result<Vec<u8>>, bool) {
    let mut open_opts = Some(AVDictionary::new(c"probesize", c"200000", 0).set(c"analyzeduration", c"100000", 0));
    let mut input_ctx = match open_input(&source, &mut open_opts) {
        Ok(ctx) => ctx,
        Err(err) => return (Err(err), false),
    };

    let selected = match find_video_stream(&input_ctx) {
        Ok(selected) => selected,
        Err(err) => return (Err(err), false),
    };

    let hw_type = if allow_hw {
        select_hw_decoder(&selected.codec_name)
    } else {
        None
    };
    let used_hw = hw_type.is_some();

    (capture_from_context(&mut input_ctx, &selected, hw_type, opts), used_hw)
}

fn open_input(source: &ScreenshotInput, open_opts: &mut Option<AVDictionary>) -> Result<AVFormatContextInput> {
    match source {
        ScreenshotInput::File(path) => {
            let path_str = path
                .to_str()
                .ok_or_else(|| Error::Other("Input path contains invalid UTF-8".into()))?;
            let c_path = CString::new(path_str)?;
            AVFormatContextInput::builder()
                .url(&c_path)
                .options(open_opts)
                .open()
                .map_err(Into::into)
        }
        ScreenshotInput::Direct(input) => open_direct_input(input.clone(), open_opts),
    }
}

fn find_video_stream(input_ctx: &AVFormatContextInput) -> Result<SelectedVideoStream> {
    for (index, stream) in input_ctx.streams().iter().enumerate() {
        let codecpar = stream.codecpar();
        if codecpar.codec_type != ffi::AVMEDIA_TYPE_VIDEO {
            continue;
        }
        if stream.disposition & ffi::AV_DISPOSITION_ATTACHED_PIC as i32 != 0 {
            continue;
        }

        let codec_name = AVCodec::find_decoder(codecpar.codec_id).map_or_else(
            || format!("{:?}", codecpar.codec_id),
            |decoder| decoder.name().to_string_lossy().into_owned(),
        );

        return Ok(SelectedVideoStream {
            index,
            codec_id: codecpar.codec_id,
            codec_name,
            time_base: stream.time_base,
        });
    }

    Err(Error::Other("No video stream found".into()))
}

fn select_hw_decoder(video_codec: &str) -> Option<HwType> {
    let caps = detect_capabilities();
    let candidates = [
        get_cuvid_decoder(video_codec, caps).map(|_| HwType::Cuda),
        (caps.has_vaapi && vaapi_decode_supported(video_codec)).then_some(HwType::Vaapi),
        (caps.has_qsv && qsv_decode_supported(video_codec)).then_some(HwType::Qsv),
        (caps.has_videotoolbox && videotoolbox_decode_supported(video_codec)).then_some(HwType::Videotoolbox),
        (caps.has_amf && amf_decode_supported(video_codec)).then_some(HwType::Amf),
    ];

    candidates
        .into_iter()
        .flatten()
        .find(|hw_type| HwAccel::get_or_create_device_ctx(*hw_type).is_some())
}

fn capture_from_context(
    input_ctx: &mut AVFormatContextInput,
    selected: &SelectedVideoStream,
    hw_type: Option<HwType>,
    opts: &VideoScreenshotOptions,
) -> Result<Vec<u8>> {
    let mut dec_ctx = open_video_decoder(input_ctx, selected, hw_type)?;
    let offset_secs = opts.offset_secs.max(0.0);

    if offset_secs > 0.0 {
        seek_input(input_ctx, offset_secs)?;
        unsafe {
            ffi::avcodec_flush_buffers(dec_ctx.as_mut_ptr());
        }
    }

    let frame = decode_frame_at_or_after(&mut dec_ctx, input_ctx, selected.index, selected.time_base, offset_secs)?;
    let frame = download_hw_frame(frame, hw_type)?;
    let filtered = scale_frame_sws(&frame, opts.format.pix_fmt(), opts.width, opts.height)?;

    encode_image_frame(
        &filtered,
        &ImageDecodeOptions {
            width: None,
            format: opts.format,
            quality: opts.quality,
        },
    )
}

fn open_video_decoder(
    input_ctx: &AVFormatContextInput,
    selected: &SelectedVideoStream,
    hw_type: Option<HwType>,
) -> Result<AVCodecContext> {
    let stream = &input_ctx.streams()[selected.index];
    let codecpar = stream.codecpar();
    let decoder = if matches!(hw_type, Some(HwType::Cuda)) {
        let caps = detect_capabilities();
        if let Some(cuvid_name) = get_cuvid_decoder(&selected.codec_name, caps) {
            let c_name = CString::new(cuvid_name)?;
            AVCodec::find_decoder_by_name(&c_name)
                .or_else(|| AVCodec::find_decoder(selected.codec_id))
                .ok_or_else(|| Error::Other(format!("No decoder found for video codec '{}'", selected.codec_name)))?
        } else {
            AVCodec::find_decoder(selected.codec_id)
                .ok_or_else(|| Error::Other(format!("No decoder found for video codec '{}'", selected.codec_name)))?
        }
    } else {
        AVCodec::find_decoder(selected.codec_id)
            .ok_or_else(|| Error::Other(format!("No decoder found for video codec '{}'", selected.codec_name)))?
    };
    let mut dec_ctx = AVCodecContext::new(&decoder);
    dec_ctx.apply_codecpar(&codecpar)?;
    dec_ctx.set_pkt_timebase(stream.time_base);
    if let Some(framerate) = stream.guess_framerate() {
        dec_ctx.set_framerate(framerate);
    }

    if let Some(hw_type) = hw_type {
        let hw = HwAccel::try_init(hw_type)
            .ok_or_else(|| Error::Other(format!("{} device is not available", hw_type.display_name())))?;
        dec_ctx.set_hw_device_ctx(hw.device_ctx.clone());
        unsafe {
            (*dec_ctx.as_mut_ptr()).pix_fmt = hw_type.pix_fmt();
            (*dec_ctx.as_mut_ptr()).hwaccel_flags |= ffi::AV_HWACCEL_FLAG_UNSAFE_OUTPUT as i32;
            (*dec_ctx.as_mut_ptr()).thread_count = 1;
        }
    } else {
        unsafe {
            (*dec_ctx.as_mut_ptr()).thread_count = 2;
        }
    }

    dec_ctx.open(None)?;
    Ok(dec_ctx)
}
fn seek_input(input_ctx: &mut AVFormatContextInput, offset_secs: f64) -> Result<()> {
    let format_start_time = if input_ctx.start_time == ffi::AV_NOPTS_VALUE {
        0
    } else {
        input_ctx.start_time
    };
    let seek_ts = (offset_secs * f64::from(ffi::AV_TIME_BASE)) as i64 + format_start_time;

    unsafe {
        let ret = ffi::avformat_seek_file(input_ctx.as_mut_ptr(), -1, i64::MIN, seek_ts, seek_ts, 0);
        if ret < 0 {
            return Err(Error::Other(format!("Failed to seek to {offset_secs}s (error {ret})")));
        }
    }

    Ok(())
}

fn decode_frame_at_or_after(
    dec_ctx: &mut AVCodecContext,
    input_ctx: &mut AVFormatContextInput,
    stream_idx: usize,
    time_base: ffi::AVRational,
    offset_secs: f64,
) -> Result<rsmpeg::avutil::AVFrame> {
    let seek_pts = compute_seek_pts(input_ctx, time_base, offset_secs);

    loop {
        let packet = match input_ctx.read_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(err) => {
                return Err(Error::Other(format!("Error reading video packet: {err:?}")));
            }
        };

        if packet.stream_index as usize != stream_idx {
            continue;
        }

        dec_ctx.send_packet(Some(&packet))?;

        loop {
            match dec_ctx.receive_frame() {
                Ok(frame) => {
                    if should_skip_frame(&frame, seek_pts) {
                        continue;
                    }
                    return Ok(frame);
                }
                Err(RsmpegError::DecoderDrainError | RsmpegError::DecoderFlushedError) => break,
                Err(err) => {
                    return Err(Error::Other(format!("Video decode error: {err:?}")));
                }
            }
        }
    }

    dec_ctx.send_packet(None).ok();
    loop {
        match dec_ctx.receive_frame() {
            Ok(frame) => {
                if should_skip_frame(&frame, seek_pts) {
                    continue;
                }
                return Ok(frame);
            }
            Err(RsmpegError::DecoderDrainError | RsmpegError::DecoderFlushedError) => break,
            Err(err) => {
                return Err(Error::Other(format!("Video decode flush error: {err:?}")));
            }
        }
    }

    Err(Error::Other(format!(
        "No decoded frame found at or after {offset_secs}s"
    )))
}

fn compute_seek_pts(input_ctx: &AVFormatContextInput, time_base: ffi::AVRational, offset_secs: f64) -> i64 {
    if offset_secs <= 0.0 || time_base.num <= 0 || time_base.den <= 0 {
        return 0;
    }

    let format_start_secs = if input_ctx.start_time == ffi::AV_NOPTS_VALUE {
        0.0
    } else {
        input_ctx.start_time as f64 / f64::from(ffi::AV_TIME_BASE)
    };
    let seek_with_offset = offset_secs + format_start_secs;

    (seek_with_offset * f64::from(time_base.den) / f64::from(time_base.num)) as i64
}

fn should_skip_frame(frame: &rsmpeg::avutil::AVFrame, seek_pts: i64) -> bool {
    seek_pts > 0 && frame.best_effort_timestamp != ffi::AV_NOPTS_VALUE && frame.best_effort_timestamp < seek_pts
}

fn download_hw_frame(frame: rsmpeg::avutil::AVFrame, hw_type: Option<HwType>) -> Result<rsmpeg::avutil::AVFrame> {
    if let Some(hw_type) = hw_type
        && frame.format == hw_type.pix_fmt()
    {
        let mut sw_frame = rsmpeg::avutil::AVFrame::new();
        sw_frame.hwframe_transfer_data(&frame)?;
        sw_frame.set_pts(frame.best_effort_timestamp);
        return Ok(sw_frame);
    }

    Ok(frame)
}

fn scale_frame_sws(
    frame: &AVFrame,
    target_pix_fmt: ffi::AVPixelFormat,
    target_width: Option<u32>,
    target_height: Option<u32>,
) -> Result<AVFrame> {
    let src_w = frame.width;
    let src_h = frame.height;
    #[allow(clippy::useless_transmute)]
    let raw_fmt = unsafe { std::mem::transmute::<i32, ffi::AVPixelFormat>(frame.format) };

    // Map deprecated YUVJ* formats to their non-J equivalents (full color range)
    let (src_fmt, src_color_range) = normalize_pix_fmt(raw_fmt);

    let (dst_w, dst_h) = compute_scale_dims(src_w, src_h, target_width, target_height);

    let mut sws_ctx = SwsContext::get_context(
        src_w,
        src_h,
        src_fmt,
        dst_w,
        dst_h,
        target_pix_fmt,
        ffi::SWS_FAST_BILINEAR,
        None,
        None,
        None,
    )
    .ok_or(Error::Other("Failed to create SwsContext".into()))?;

    // Set color range so swscale doesn't guess wrong
    unsafe {
        ffi::sws_setColorspaceDetails(
            sws_ctx.as_mut_ptr(),
            ffi::sws_getCoefficients(ffi::SWS_CS_DEFAULT as i32),
            src_color_range as i32,
            ffi::sws_getCoefficients(ffi::SWS_CS_DEFAULT as i32),
            0, // output: limited range (TV)
            0,
            1 << 16,
            1 << 16,
        );
    }

    let mut dst_frame = AVFrame::new();
    unsafe {
        (*dst_frame.as_mut_ptr()).width = dst_w;
        (*dst_frame.as_mut_ptr()).height = dst_h;
        #[allow(clippy::unnecessary_cast)]
        {
            (*dst_frame.as_mut_ptr()).format = target_pix_fmt as i32;
        }
    }
    dst_frame.alloc_buffer()?;

    sws_ctx.scale_frame(frame, 0, src_h, &mut dst_frame)?;

    Ok(dst_frame)
}

/// Map deprecated YUVJ* pixel formats to their YUV* equivalents.
/// Returns `(canonical_fmt, is_full_range)`.
fn normalize_pix_fmt(fmt: ffi::AVPixelFormat) -> (ffi::AVPixelFormat, u32) {
    match fmt {
        ffi::AV_PIX_FMT_YUVJ420P => (ffi::AV_PIX_FMT_YUV420P, ffi::AVCOL_RANGE_JPEG),
        ffi::AV_PIX_FMT_YUVJ422P => (ffi::AV_PIX_FMT_YUV422P, ffi::AVCOL_RANGE_JPEG),
        ffi::AV_PIX_FMT_YUVJ444P => (ffi::AV_PIX_FMT_YUV444P, ffi::AVCOL_RANGE_JPEG),
        ffi::AV_PIX_FMT_YUVJ440P => (ffi::AV_PIX_FMT_YUV440P, ffi::AVCOL_RANGE_JPEG),
        other => (other, ffi::AVCOL_RANGE_MPEG),
    }
}

fn compute_scale_dims(src_w: i32, src_h: i32, target_width: Option<u32>, target_height: Option<u32>) -> (i32, i32) {
    match (target_width, target_height) {
        (Some(w), Some(h)) => (w as i32, h as i32),
        (Some(w), None) => {
            let w = w as i32;
            let h = if src_w > 0 {
                ((src_h * w + src_w / 2) / src_w).max(1)
            } else {
                src_h
            };
            (w, (h + 1) & !1)
        }
        (None, Some(h)) => {
            let h = h as i32;
            let w = if src_h > 0 {
                ((src_w * h + src_h / 2) / src_h).max(1)
            } else {
                src_w
            };
            ((w + 1) & !1, h)
        }
        (None, None) => (src_w, src_h),
    }
}
