//! Media Foundation's hardware H.264 encoder as an Export encoder: [`factory`] is registered behind
//! NVENC and in front of the software encoder (`filmcraft_export::register_encoder`), so an export
//! on an Intel (Quick Sync), AMD or Qualcomm GPU, or on NVIDIA when NVENC declines, encodes on the
//! GPU's video engine.
//!
//! It takes an export only when Export ▸ Hardware encoding is Auto and the settings are ones it
//! writes exactly as asked: H.264 in MP4 / MOV, SDR BT.709, one pass (CBR or VBR), progressive,
//! square pixels, even sizes. Anything else, or no hardware encoder MFT that takes the stream,
//! returns `None` and the software encoder runs, exactly as before. Like NVENC, a hardware encoder
//! that fails in the middle of an export ends it with an error naming it (the two encoders write
//! different streams, so it cannot hand over).
//!
//! `FILMCRAFT_MF_SOFTWARE_ENCODER=1` lets Microsoft's software H.264 MFT stand in for the hardware
//! one (CI runners have no GPU): the same code path, end to end.

use filmcraft_export::{
    BitrateMode, ColorSignal, EncodedPacket, EncoderFrame, ExportError, ExportSettings, FieldOrder, Format, H264Pass, H264Profile, HardwareEncoding, Result,
    VideoEncoder,
};
use filmcraft_isobmff::{AvcConfig, SampleEntry};
use filmcraft_time::FrameRate;

use super::encode::{EncConfig, MfEncoder, Packet};
use super::mft::{MfApi, api};
use crate::nvenc::{Codec, annex_b_nals, annex_b_to_length_prefixed, nal_type};

/// The export muxes after this many pictures (`filmcraft_export`'s interleave): the parameter
/// sets must be known by then.
const PARAMETER_SETS_BY: u64 = 15;

struct MfExportEncoder {
    enc: MfEncoder,
    api: &'static MfApi,
    w: u32,
    h: u32,
    rate: FrameRate,
    nv12: Vec<u8>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    last_time: Option<i64>,
}

/// Whether Microsoft's software MFT may stand in (tests and CI only).
fn software_allowed() -> bool {
    std::env::var_os("FILMCRAFT_MF_SOFTWARE_ENCODER").is_some_and(|v| v == "1")
}

impl MfExportEncoder {
    /// SPS / PPS from Annex B NAL units, where the encoder put them.
    fn learn_parameter_sets(&mut self, annex_b: &[u8]) {
        for nal in annex_b_nals(annex_b) {
            match nal_type(Codec::H264, nal) {
                Some(7) if self.sps.is_none() => self.sps = Some(nal.to_vec()),
                Some(8) if self.pps.is_none() => self.pps = Some(nal.to_vec()),
                _ => {}
            }
        }
    }

    fn packets(&mut self, ps: Vec<Packet>) -> Result<Vec<EncodedPacket>> {
        let duration = u32::try_from(self.rate.den).unwrap_or(u32::MAX);
        let mut out = Vec::with_capacity(ps.len());
        for p in ps {
            self.learn_parameter_sets(&p.data);
            // no B-frames were asked for: pictures must come out in order
            if self.last_time.is_some_and(|t| p.time < t) {
                return Err(ExportError::Encode(format!(
                    "{}: the encoder reordered pictures (B-frames); turn Export ▸ Hardware encoding off",
                    self.enc.name()
                )));
            }
            self.last_time = Some(p.time);
            let data = annex_b_to_length_prefixed(&p.data).map_err(|e| ExportError::Encode(format!("{}: {e}", self.enc.name())))?;
            out.push(EncodedPacket { data, key: p.key, duration, composition_offset: 0 });
        }
        Ok(out)
    }
}

impl VideoEncoder for MfExportEncoder {
    fn sample_entry(&self) -> SampleEntry {
        let (w, h) = (u16::try_from(self.w).unwrap_or(u16::MAX), u16::try_from(self.h).unwrap_or(u16::MAX));
        let (sps, pps) = (self.sps.clone().unwrap_or_default(), self.pps.clone().unwrap_or_default());
        SampleEntry::avc(AvcConfig::new(vec![sps], vec![pps], 4), w, h)
    }

    fn timescale(&self) -> u32 {
        u32::try_from(self.rate.num).unwrap_or(u32::MAX)
    }

    fn encode(&mut self, f: &EncoderFrame) -> Result<Vec<EncodedPacket>> {
        if f.width != self.w || f.height != self.h {
            return Err(ExportError::Encode(format!("{}: a {}x{} picture for a {}x{} encoder", self.enc.name(), f.width, f.height, self.w, self.h)));
        }
        if f.hdr.is_some() {
            return Err(ExportError::Unsupported("the Media Foundation encoder takes SDR pictures only".into()));
        }
        let (w, h) = (self.w as usize, self.h as usize);
        let mut nv12 = std::mem::take(&mut self.nv12);
        filmcraft_export::timed(filmcraft_export::Stage::Convert, || filmcraft_export::rgba_to_nv12(f.rgba, w, h, &mut nv12));
        let out = self.encode_picture(&nv12, f.index);
        self.nv12 = nv12;
        out
    }

    fn takes_nv12(&self) -> bool {
        true
    }

    fn encode_nv12(&mut self, width: u32, height: u32, nv12: &[u8], index: u64) -> Result<Vec<EncodedPacket>> {
        if width != self.w || height != self.h {
            return Err(ExportError::Encode(format!("{}: a {width}x{height} picture for a {}x{} encoder", self.enc.name(), self.w, self.h)));
        }
        let want = (self.w as usize) * (self.h as usize) * 3 / 2;
        if nv12.len() != want {
            return Err(ExportError::Encode(format!("{}: {} bytes of NV12 for {want}", self.enc.name(), nv12.len())));
        }
        self.encode_picture(nv12, index)
    }

    fn flush(&mut self) -> Result<Vec<EncodedPacket>> {
        let ps = self.enc.flush(self.api).map_err(|e| ExportError::Encode(e.to_string()))?;
        self.packets(ps)
    }
}

impl MfExportEncoder {
    /// One NV12 picture into the encoder, and the packets that came out.
    fn encode_picture(&mut self, nv12: &[u8], index: u64) -> Result<Vec<EncodedPacket>> {
        let hint = " (turn Export ▸ Hardware encoding off to use the software encoder)";
        let ps = self.enc.encode(self.api, nv12, index).map_err(|e| ExportError::Encode(format!("{e}{hint}")))?;
        filmcraft_export::note_hw_encode_frame();
        let out = self.packets(ps)?;
        if self.sps.is_none()
            && let Some(header) = self.enc.sequence_header()
        {
            self.learn_parameter_sets(&header);
        }
        if index >= PARAMETER_SETS_BY && (self.sps.is_none() || self.pps.is_none()) {
            return Err(ExportError::Encode(format!("{}: no SPS / PPS after {} pictures{hint}", self.enc.name(), index + 1)));
        }
        Ok(out)
    }
}

/// Why this encoder does not take the export (the software encoder does), or its configuration.
fn config(format: Format, w: u32, h: u32, rate: FrameRate, s: &ExportSettings) -> std::result::Result<EncConfig, String> {
    if format != Format::H264 || s.format.is_mxf() {
        return Err("only MP4 / MOV H.264 exports".into());
    }
    if s.signal != ColorSignal::default() {
        return Err(format!("the colour signal {:?} (this encoder writes BT.709 SDR)", s.signal));
    }
    if !matches!(s.h264_pass, H264Pass::Single) || s.bitrate_mode == BitrateMode::Vbr2Pass {
        return Err("two-pass VBR".into());
    }
    if s.bitrate_mode == BitrateMode::Crf {
        return Err("CRF (constant quality)".into());
    }
    if s.pixel_aspect.is_some_and(|(n, d)| n != d) {
        return Err("non-square pixels".into());
    }
    if s.field_order != FieldOrder::Progressive {
        return Err("interlaced output".into());
    }
    if w < 64 || h < 64 || !w.is_multiple_of(2) || !h.is_multiple_of(2) || w > 8192 || h > 8192 {
        return Err(format!("{w}x{h} (even sizes from 64 to 8192)"));
    }
    let (Ok(num), Ok(den)) = (u32::try_from(rate.num), u32::try_from(rate.den)) else {
        return Err("frame rate".into());
    };
    if num == 0 || den == 0 {
        return Err("frame rate".into());
    }
    let kbps = s.bitrate_kbps.clamp(100, 800_000);
    Ok(EncConfig {
        width: w,
        height: h,
        fps: (num, den),
        bitrate_kbps: kbps,
        max_bitrate_kbps: s.max_bitrate_kbps.filter(|m| *m >= kbps).unwrap_or_else(|| kbps.saturating_add(kbps / 2)),
        cbr: s.bitrate_mode == BitrateMode::Cbr,
        keyint: s.keyframe_distance.filter(|k| *k > 0).unwrap_or_else(|| (f64::from(num) / f64::from(den) * 2.0).round().clamp(1.0, 600.0) as u32),
        profile: match s.h264_profile {
            H264Profile::Baseline => 66,
            H264Profile::Main => 77,
            H264Profile::High => 100,
        },
    })
}

/// The Export encoder factory (see the module documentation).
pub fn factory(format: Format, w: u32, h: u32, rate: FrameRate, s: &ExportSettings) -> Option<Result<Box<dyn VideoEncoder>>> {
    factory_with(format, w, h, rate, s, |cfg| {
        let api = api()?;
        MfEncoder::open(api, cfg, software_allowed()).map(|e| (api, e))
    })
}

/// [`factory`] with the Media Foundation call (`open`) passed in, so tests can stand in for it.
fn factory_with(
    format: Format,
    w: u32,
    h: u32,
    rate: FrameRate,
    s: &ExportSettings,
    open: impl FnOnce(&EncConfig) -> std::result::Result<(&'static MfApi, MfEncoder), String>,
) -> Option<Result<Box<dyn VideoEncoder>>> {
    if s.hardware_encoding != HardwareEncoding::Auto || format != Format::H264 {
        return None;
    }
    let declined = |why: &str| {
        log::info!("Media Foundation hardware encoding declined: {why}");
        filmcraft_export::note_hw_encode_declined();
        None
    };
    let cfg = match config(format, w, h, rate, s) {
        Ok(c) => c,
        Err(why) => return declined(&why),
    };
    match open(&cfg) {
        Ok((api, enc)) => {
            filmcraft_export::note_hw_encode_session();
            let mut e = MfExportEncoder { enc, api, w, h, rate, nv12: Vec::new(), sps: None, pps: None, last_time: None };
            if let Some(header) = e.enc.sequence_header() {
                e.learn_parameter_sets(&header);
            }
            Some(Ok(Box::new(e)))
        }
        Err(why) => declined(&why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_and_unsupported_settings_never_open_an_encoder() {
        let declined = |format, w, h, s: &ExportSettings| {
            assert!(factory_with(format, w, h, FrameRate::FPS_24, s, |_| panic!("must not open an encoder")).is_none());
        };
        let auto = ExportSettings { hardware_encoding: HardwareEncoding::Auto, ..Default::default() };
        declined(Format::H264, 640, 360, &ExportSettings { hardware_encoding: HardwareEncoding::Off, ..Default::default() });
        declined(Format::Hevc, 640, 360, &auto);
        declined(Format::H264, 641, 360, &auto);
        declined(Format::H264, 32, 32, &auto);
        declined(Format::H264, 640, 360, &ExportSettings { bitrate_mode: BitrateMode::Crf, ..auto.clone() });
        declined(Format::H264, 640, 360, &ExportSettings { bitrate_mode: BitrateMode::Vbr2Pass, ..auto.clone() });
        declined(Format::H264, 640, 360, &ExportSettings { pixel_aspect: Some((4, 3)), ..auto.clone() });
        declined(Format::H264, 640, 360, &ExportSettings { field_order: FieldOrder::UpperFirst, ..auto.clone() });
    }

    #[test]
    fn a_configuration_follows_the_settings() {
        let s = ExportSettings { hardware_encoding: HardwareEncoding::Auto, bitrate_kbps: 12_000, ..Default::default() };
        let c = config(Format::H264, 1080, 1920, FrameRate::FPS_30, &s).unwrap();
        assert_eq!((c.width, c.height, c.fps, c.bitrate_kbps, c.max_bitrate_kbps, c.keyint), (1080, 1920, (30, 1), 12_000, 18_000, 60));
        assert_eq!(c.profile, 100);
        let mut out = Vec::new();
        to_nv12(&[1, 2, 3, 4], &[10], &[20], &mut out);
        assert_eq!(out, [1, 2, 3, 4, 10, 20]);
    }
}
