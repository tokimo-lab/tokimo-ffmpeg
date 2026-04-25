//! DTS estimator for stream-copy packets.
//!
//! Mirrors `FFmpeg` CLI's `DemuxStream.{dts, next_dts}` in `ffmpeg_demux.c`.
//! When the container doesn't provide DTS (`AV_NOPTS_VALUE`) — common after seeks
//! — this fills in an estimate so the muxer never receives unset timestamps.

use rsmpeg::ffi;

pub(crate) struct DtsEstimator {
    /// Predicted DTS of the next packet (input timebase units).
    next_dts: i64,
    /// Whether we've seen the first packet yet.
    saw_first: bool,
    /// Codec type (video or audio).
    codec_type: ffi::AVMediaType,
    /// Input stream `time_base` for duration calculations.
    time_base: ffi::AVRational,
    /// Video: framerate for duration estimation.
    framerate: ffi::AVRational,
    /// Video: B-frame depth (`video_delay`) — used to start DTS negative.
    video_delay: i32,
    /// Audio: `sample_rate` and `frame_size` for duration estimation.
    sample_rate: i32,
    frame_size: i32,
}

impl DtsEstimator {
    pub fn new(stream: &ffi::AVStream, par: &ffi::AVCodecParameters) -> Self {
        Self {
            next_dts: ffi::AV_NOPTS_VALUE,
            saw_first: false,
            codec_type: par.codec_type,
            time_base: stream.time_base,
            framerate: if par.framerate.num != 0 {
                par.framerate
            } else {
                stream.avg_frame_rate
            },
            video_delay: par.video_delay,
            sample_rate: par.sample_rate,
            frame_size: par.frame_size,
        }
    }

    /// Compute the frame duration in input timebase units.
    fn frame_dur(&self) -> i64 {
        if self.codec_type == ffi::AVMEDIA_TYPE_VIDEO && self.framerate.num > 0 && self.framerate.den > 0 {
            Self::rescale(
                1,
                ffi::AVRational {
                    num: self.framerate.den,
                    den: self.framerate.num,
                },
                self.time_base,
            )
        } else if self.codec_type == ffi::AVMEDIA_TYPE_AUDIO && self.sample_rate > 0 && self.frame_size > 0 {
            Self::rescale(
                i64::from(self.frame_size),
                ffi::AVRational {
                    num: 1,
                    den: self.sample_rate,
                },
                self.time_base,
            )
        } else {
            0
        }
    }

    /// Fix missing DTS/PTS on a packet before it's written to the muxer.
    /// Operates in input timebase — call before `rescale_ts`.
    ///
    /// Mirrors `FFmpeg` CLI's DTS estimation in `ffmpeg_demux.c`:
    /// - For video with B-frames, starts DTS at `-video_delay * frame_dur` so
    ///   estimated DTS values are negative.  The muxer's monotonic-DTS check
    ///   skips packets with `dts < 0`, and `avoid_negative_ts = make_non_negative`
    ///   shifts everything forward in the output.
    /// - When the container/parser starts providing valid DTS, we switch to
    ///   tracking it and only estimate for subsequent gaps.
    pub fn fix_timestamps(&mut self, pkt: &mut rsmpeg::avcodec::AVPacket) {
        // ── First packet: bootstrap ──────────────────────────────────────
        if !self.saw_first {
            self.saw_first = true;
            let fdur = self.frame_dur();
            if pkt.dts != ffi::AV_NOPTS_VALUE {
                self.next_dts = pkt.dts;
            } else if pkt.pts != ffi::AV_NOPTS_VALUE {
                // Mirrors ffmpeg_demux.c: first_dts = -video_delay/fps + pts
                let delay_offset = if self.codec_type == ffi::AVMEDIA_TYPE_VIDEO && fdur > 0 {
                    -i64::from(self.video_delay) * fdur
                } else {
                    0
                };
                self.next_dts = pkt.pts + delay_offset;
            } else {
                self.next_dts = 0;
            }
        }

        // ── Update from container or apply estimate ─────────────────────
        if pkt.dts == ffi::AV_NOPTS_VALUE {
            // Missing DTS — fill from our estimate.
            pkt.set_dts(self.next_dts);
        } else {
            // Container provides DTS — but enforce monotonic increase.
            // After seeks in MKV with B-frames, the demuxer may emit a
            // few packets whose DTS goes slightly backwards due to
            // reordering.  The HLS/mpegts muxer rejects non-monotonic
            // DTS, which would kill the entire transcode worker.
            if self.next_dts != ffi::AV_NOPTS_VALUE && pkt.dts < self.next_dts {
                pkt.set_dts(self.next_dts);
            } else {
                self.next_dts = pkt.dts;
            }
        }

        // ── Fix missing PTS & ensure PTS >= DTS ─────────────────────────
        if pkt.pts == ffi::AV_NOPTS_VALUE {
            pkt.set_pts(self.next_dts);
        } else if pkt.pts < pkt.dts {
            // PTS must never be less than DTS (required by mpegts muxer).
            pkt.set_pts(pkt.dts);
        }

        // ── Advance next_dts for the following packet ───────────────────
        self.advance_next_dts(pkt);
    }

    fn advance_next_dts(&mut self, pkt: &rsmpeg::avcodec::AVPacket) {
        let fdur = self.frame_dur();
        if fdur > 0 {
            self.next_dts += fdur;
        } else if pkt.duration > 0 {
            self.next_dts += pkt.duration;
        }
    }

    /// `av_rescale_q` equivalent: value * from / to
    fn rescale(value: i64, from: ffi::AVRational, to: ffi::AVRational) -> i64 {
        // Use i128 to avoid overflow, matching our scheduler's approach
        let num = i128::from(value) * i128::from(from.num) * i128::from(to.den);
        let den = i128::from(from.den) * i128::from(to.num);
        if den == 0 {
            return 0;
        }
        (num / den) as i64
    }
}
