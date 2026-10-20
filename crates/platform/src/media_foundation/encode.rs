//! The Media Foundation side of the Windows H.264 encoder: finding a hardware H.264 encoder MFT
//! (Intel Quick Sync, AMD AMF / VCN, NVIDIA's, Qualcomm's...), configuring it for one export and
//! driving it one picture at a time, synchronous or asynchronous.
//!
//! Pictures go in as system-memory NV12 (the export's own BT.709 limited-range conversion, so the
//! colours are the software encoder's); the driver uploads them. The encoder runs without B-frames,
//! so packets come out in presentation order. Asynchronous MFTs (every hardware encoder) are driven
//! by polling their events with a deadline: a driver that stops answering ends the export with an
//! error instead of hanging it.
//!
//! FFI module (docs/adr/0001-platform-ffi.md): every `unsafe` block has a `// SAFETY:` comment,
//! the public items are safe and return `Result<_, String>`.

use std::mem::ManuallyDrop;
use std::time::{Duration, Instant};

use windows::Win32::Media::MediaFoundation::{
    CODECAPI_AVEncCommonMaxBitRate, CODECAPI_AVEncCommonMeanBitRate, CODECAPI_AVEncCommonRateControlMode, CODECAPI_AVEncMPVDefaultBPictureCount,
    CODECAPI_AVEncMPVGOPSize, ICodecAPI, IMFActivate, IMFMediaEventGenerator, IMFMediaType, IMFSample, IMFTransform, MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS,
    METransformDrainComplete, METransformHaveOutput, METransformNeedInput, MF_E_NO_EVENTS_AVAILABLE, MF_E_NO_MORE_TYPES, MF_E_NOTACCEPTING,
    MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE, MF_EVENT_FLAG_NO_WAIT, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
    MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_MPEG_SEQUENCE_HEADER, MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_MT_TRANSFER_FUNCTION,
    MF_MT_VIDEO_NOMINAL_RANGE, MF_MT_VIDEO_PRIMARIES, MF_MT_YUV_MATRIX, MF_TRANSFORM_ASYNC, MF_TRANSFORM_ASYNC_UNLOCK, MFMediaType_Video,
    MFNominalRange_16_235, MFSampleExtension_CleanPoint, MFT_ENUM_HARDWARE_URL_Attribute, MFT_MESSAGE_COMMAND_DRAIN, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
    MFT_MESSAGE_NOTIFY_END_OF_STREAM, MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFVideoFormat_H264,
    MFVideoFormat_NV12, MFVideoInterlace_Progressive, MFVideoPrimaries_BT709, MFVideoTransFunc_709, MFVideoTransferMatrix_BT709,
    eAVEncCommonRateControlMode_CBR, eAVEncCommonRateControlMode_PeakConstrainedVBR,
};
use windows::Win32::System::Variant::VARIANT;
use windows::core::Interface;

use super::gpu::ensure_com;
use super::mft::{MfApi, friendly_name};

/// How long a driver may take to answer before the export ends with an error.
const DEADLINE: Duration = Duration::from_secs(20);
/// Upper bound on the packets taken per call: a misbehaving MFT cannot make a call loop.
const MAX_OUTPUTS_PER_CALL: usize = 256;
/// Media Foundation times are in 100 ns units.
const HNS: i64 = 10_000_000;

/// What the encoder is asked to make.
#[derive(Clone, Debug, PartialEq)]
pub struct EncConfig {
    pub width: u32,
    pub height: u32,
    /// Frames per second as num / den.
    pub fps: (u32, u32),
    pub bitrate_kbps: u32,
    pub max_bitrate_kbps: u32,
    pub cbr: bool,
    /// Frames between key frames.
    pub keyint: u32,
    /// `eAVEncH264VProfile`: 66 Baseline, 77 Main, 100 High.
    pub profile: u32,
}

/// One compressed picture: Annex B NAL units.
pub struct Packet {
    pub data: Vec<u8>,
    pub key: bool,
    /// Presentation time, 100 ns units.
    pub time: i64,
}

/// A configured H.264 encoder MFT.
pub struct MfEncoder {
    transform: IMFTransform,
    /// Asynchronous MFTs: their event queue.
    events: Option<IMFMediaEventGenerator>,
    name: String,
    /// The MFT allocates its output samples; otherwise each output gets one of `out_size` bytes.
    provides_samples: bool,
    out_size: u32,
    /// Input requests (`METransformNeedInput`) not answered yet.
    wanted: u32,
    /// Output ready (`METransformHaveOutput`) not taken yet.
    ready: u32,
    width: u32,
    height: u32,
    duration: i64,
}

// SAFETY: encoder MFTs are free-threaded COM objects; an `MfEncoder` is used through `&mut` by the
// one export job that owns it, on whichever thread runs that job (COM is initialised there first).
unsafe impl Send for MfEncoder {}

impl MfEncoder {
    /// The first encoder MFT that takes `cfg`: hardware ones only, unless `software` also allows
    /// Microsoft's software encoder (tests; the software H.264 encoder is the real fallback).
    pub fn open(api: &MfApi, cfg: &EncConfig, software: bool) -> Result<MfEncoder, String> {
        ensure_com()?;
        let candidates = api.encoders(MFVideoFormat_NV12, MFVideoFormat_H264)?;
        let mut reasons = Vec::new();
        for a in candidates {
            let name = friendly_name(&a).unwrap_or_else(|| "an H.264 encoder MFT".into());
            // SAFETY: a plain COM call on a live interface.
            let hardware = unsafe { a.GetStringLength(&MFT_ENUM_HARDWARE_URL_Attribute) }.is_ok();
            if !hardware && !software {
                reasons.push(format!("{name}: not a hardware encoder"));
                continue;
            }
            match Self::configure(api, &a, &name, cfg) {
                Ok(e) => {
                    log::info!("Media Foundation H.264 encoder: {name} (hardware {hardware})");
                    return Ok(e);
                }
                Err(e) => reasons.push(e),
            }
        }
        if reasons.is_empty() {
            return Err("no H.264 encoder MFT on this system".into());
        }
        Err(reasons.join("; "))
    }

    /// The encoder's name, for logs and messages.
    pub fn name(&self) -> &str {
        &self.name
    }

    fn configure(api: &MfApi, activate: &IMFActivate, name: &str, cfg: &EncConfig) -> Result<MfEncoder, String> {
        // SAFETY: plain COM calls on live interfaces.
        let transform: IMFTransform = unsafe { activate.ActivateObject() }.map_err(|e| format!("{name}: cannot be created: {e}"))?;
        // SAFETY: plain COM calls on live interfaces.
        let events = unsafe {
            let attrs = transform.GetAttributes().map_err(|e| format!("{name}: no attributes: {e}"))?;
            if attrs.GetUINT32(&MF_TRANSFORM_ASYNC).unwrap_or(0) != 0 {
                attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1).map_err(|e| format!("{name}: cannot be unlocked: {e}"))?;
                Some(transform.cast::<IMFMediaEventGenerator>().map_err(|e| format!("{name}: an asynchronous MFT without events: {e}"))?)
            } else {
                None
            }
        };
        let duration = (HNS * i64::from(cfg.fps.1) / i64::from(cfg.fps.0.max(1))).max(1);
        let mut enc = MfEncoder {
            transform,
            events,
            name: name.to_string(),
            provides_samples: false,
            out_size: 0,
            wanted: 0,
            ready: 0,
            width: cfg.width,
            height: cfg.height,
            duration,
        };
        enc.set_codec_api(cfg);
        // the output (H.264) type first, then the input: encoders size their input from the output
        let out = api.media_type()?;
        let common = |t: &IMFMediaType| -> windows::core::Result<()> {
            // SAFETY: plain COM calls on live interfaces.
            unsafe {
                t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                t.SetUINT64(&MF_MT_FRAME_SIZE, (u64::from(cfg.width) << 32) | u64::from(cfg.height))?;
                t.SetUINT64(&MF_MT_FRAME_RATE, (u64::from(cfg.fps.0) << 32) | u64::from(cfg.fps.1))?;
                t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
                t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                // BT.709, limited range: what the NV12 is, and what the stream's VUI should say
                t.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)?;
                t.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)?;
                t.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
                t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)
            }
        };
        // SAFETY: plain COM calls on live interfaces.
        unsafe {
            common(&out).map_err(|e| format!("{name}: {e}"))?;
            out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264).map_err(|e| e.to_string())?;
            out.SetUINT32(&MF_MT_AVG_BITRATE, cfg.bitrate_kbps.saturating_mul(1000)).map_err(|e| e.to_string())?;
            out.SetUINT32(&MF_MT_MPEG2_PROFILE, cfg.profile).map_err(|e| e.to_string())?;
            enc.transform.SetOutputType(0, &out, 0).map_err(|e| format!("{name}: does not take this output ({}x{}): {e}", cfg.width, cfg.height))?;
        }
        enc.set_input(api, &common)?;
        // SAFETY: plain COM calls on live interfaces.
        unsafe {
            let info = enc.transform.GetOutputStreamInfo(0).map_err(|e| format!("{name}: no output stream info: {e}"))?;
            enc.provides_samples = info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0;
            enc.out_size = info.cbSize.max(cfg.width.saturating_mul(cfg.height).saturating_mul(3) / 2).max(1 << 16);
            enc.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0).map_err(|e| format!("{name}: {e}"))?;
            enc.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0).map_err(|e| format!("{name}: {e}"))?;
        }
        Ok(enc)
    }

    /// Rate control, key frame distance and no B-frames, through `ICodecAPI` where the MFT has it.
    /// Before the media types: some encoders read them only then. A setting an encoder refuses is
    /// logged; one that has no `ICodecAPI` at all keeps its defaults (and the bitrate of its type).
    fn set_codec_api(&self, cfg: &EncConfig) {
        let Ok(api) = self.transform.cast::<ICodecAPI>() else {
            log::info!("{}: no ICodecAPI; encoder defaults", self.name);
            return;
        };
        let mode = if cfg.cbr { eAVEncCommonRateControlMode_CBR } else { eAVEncCommonRateControlMode_PeakConstrainedVBR };
        let settings = [
            (CODECAPI_AVEncCommonRateControlMode, mode.0 as u32, "rate control"),
            (CODECAPI_AVEncCommonMeanBitRate, cfg.bitrate_kbps.saturating_mul(1000), "bitrate"),
            (CODECAPI_AVEncCommonMaxBitRate, cfg.max_bitrate_kbps.saturating_mul(1000), "peak bitrate"),
            (CODECAPI_AVEncMPVGOPSize, cfg.keyint, "key frame distance"),
            (CODECAPI_AVEncMPVDefaultBPictureCount, 0, "B-frames"),
        ];
        for (key, value, what) in settings {
            let v = VARIANT::from(value);
            // SAFETY: a plain COM call; the GUID and the VARIANT live across it.
            if let Err(e) = unsafe { api.SetValue(&key, &v) } {
                log::info!("{}: {what} not set ({e})", self.name);
            }
        }
    }

    /// The NV12 input type: as given, or the MFT's own NV12 type with this size and rate.
    fn set_input(&self, api: &MfApi, common: &dyn Fn(&IMFMediaType) -> windows::core::Result<()>) -> Result<(), String> {
        let name = &self.name;
        let input = api.media_type()?;
        // SAFETY: plain COM calls on live interfaces.
        unsafe {
            common(&input).map_err(|e| format!("{name}: {e}"))?;
            input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12).map_err(|e| e.to_string())?;
            if self.transform.SetInputType(0, &input, 0).is_ok() {
                return Ok(());
            }
            for i in 0..64 {
                match self.transform.GetInputAvailableType(0, i) {
                    Ok(t) => {
                        if t.GetGUID(&MF_MT_SUBTYPE).ok() == Some(MFVideoFormat_NV12) {
                            common(&t).map_err(|e| format!("{name}: {e}"))?;
                            return self.transform.SetInputType(0, &t, 0).map_err(|e| format!("{name}: refused NV12 input: {e}"));
                        }
                    }
                    Err(e) if e.code() == MF_E_NO_MORE_TYPES => break,
                    Err(e) => return Err(format!("{name}: no input types: {e}")),
                }
            }
        }
        Err(format!("{name}: does not take NV12 pictures"))
    }

    /// The SPS and PPS (Annex B) the encoder published on its output type, when it did.
    pub fn sequence_header(&self) -> Option<Vec<u8>> {
        // SAFETY: plain COM calls on live interfaces; `GetBlobSize` bounds the copy.
        unsafe {
            let t = self.transform.GetOutputCurrentType(0).ok()?;
            let n = t.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER).ok()? as usize;
            let mut buf = vec![0u8; n];
            let mut len = 0u32;
            t.GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut buf, Some(&mut len)).ok()?;
            buf.truncate(len as usize);
            (!buf.is_empty()).then_some(buf)
        }
    }

    /// Encode picture `index` (NV12: `width × height` luma, then interleaved Cb Cr at half size):
    /// the packets that came out meanwhile.
    pub fn encode(&mut self, api: &MfApi, nv12: &[u8], index: u64) -> Result<Vec<Packet>, String> {
        ensure_com()?;
        let need = (self.width as usize).saturating_mul(self.height as usize).saturating_mul(3) / 2;
        if nv12.len() != need {
            return Err(format!("{}: a picture of {} bytes for {}x{} NV12", self.name, nv12.len(), self.width, self.height));
        }
        let sample = self.input_sample(api, nv12, index)?;
        let mut out = Vec::new();
        let start = Instant::now();
        if self.events.is_some() {
            // asynchronous: wait for the MFT to ask for input, taking any output meanwhile
            while self.wanted == 0 {
                self.pump(api, &mut out, start, true)?;
            }
            self.wanted -= 1;
            self.process_input(&sample)?;
            self.pump(api, &mut out, start, false)?;
        } else {
            loop {
                // SAFETY: plain COM call on live interfaces.
                match unsafe { self.transform.ProcessInput(0, &sample, 0) } {
                    Ok(()) => break,
                    Err(e) if e.code() == MF_E_NOTACCEPTING => {
                        let before = out.len();
                        self.drain_sync(api, &mut out)?;
                        if out.len() == before && start.elapsed() > DEADLINE {
                            return Err(format!("{}: takes no more input and gives no output", self.name));
                        }
                    }
                    Err(e) => return Err(format!("{}: ProcessInput failed: {e}", self.name)),
                }
            }
            self.drain_sync(api, &mut out)?;
        }
        Ok(out)
    }

    /// The pictures held back: end of stream, drain, and every packet still inside.
    pub fn flush(&mut self, api: &MfApi) -> Result<Vec<Packet>, String> {
        ensure_com()?;
        // SAFETY: plain COM calls on a live interface.
        unsafe {
            self.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0).map_err(|e| format!("{}: {e}", self.name))?;
            self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0).map_err(|e| format!("{}: drain failed: {e}", self.name))?;
        }
        let mut out = Vec::new();
        if self.events.is_none() {
            self.drain_sync(api, &mut out)?;
            return Ok(out);
        }
        let start = Instant::now();
        while out.len() < MAX_OUTPUTS_PER_CALL * 4 {
            match self.next_event(start, true)? {
                Some(t) if t == METransformHaveOutput.0 as u32 => {
                    if let Some(p) = self.output(api)? {
                        out.push(p);
                    }
                }
                Some(t) if t == METransformDrainComplete.0 as u32 => break,
                // input requests after the end of the stream are not answered
                _ => {}
            }
        }
        Ok(out)
    }

    fn input_sample(&self, api: &MfApi, nv12: &[u8], index: u64) -> Result<IMFSample, String> {
        let len = u32::try_from(nv12.len()).map_err(|_| "picture is too large".to_string())?;
        let (sample, buffer) = api.empty_sample(len)?;
        let time = i64::try_from(index).unwrap_or(i64::MAX / 2).saturating_mul(self.duration);
        // SAFETY: `Lock` gives a writable block of at least `max` bytes valid until `Unlock`; the
        // copy stays within `len <= max` bytes and the buffer is unlocked before it is used.
        unsafe {
            let (mut ptr, mut max) = (std::ptr::null_mut::<u8>(), 0u32);
            buffer.Lock(&mut ptr, Some(&mut max), None).map_err(|e| format!("cannot lock the input buffer: {e}"))?;
            if ptr.is_null() || max < len {
                let _ = buffer.Unlock();
                return Err("input buffer is too small".into());
            }
            std::ptr::copy_nonoverlapping(nv12.as_ptr(), ptr, nv12.len());
            buffer.Unlock().map_err(|e| format!("cannot unlock the input buffer: {e}"))?;
            buffer.SetCurrentLength(len).map_err(|e| format!("cannot size the input buffer: {e}"))?;
            sample.SetSampleTime(time).map_err(|e| format!("cannot timestamp the input: {e}"))?;
            sample.SetSampleDuration(self.duration).map_err(|e| format!("cannot time the input: {e}"))?;
        }
        Ok(sample)
    }

    fn process_input(&self, sample: &IMFSample) -> Result<(), String> {
        // SAFETY: plain COM call on live interfaces.
        unsafe { self.transform.ProcessInput(0, sample, 0) }.map_err(|e| format!("{}: ProcessInput failed: {e}", self.name))
    }

    /// Asynchronous MFTs: handle queued events (output is taken at once). `block`: wait for at
    /// least one, up to the deadline.
    fn pump(&mut self, api: &MfApi, out: &mut Vec<Packet>, start: Instant, block: bool) -> Result<(), String> {
        let mut wait = block;
        for _ in 0..MAX_OUTPUTS_PER_CALL {
            let Some(t) = self.next_event(start, wait)? else { return Ok(()) };
            wait = false;
            if t == METransformNeedInput.0 as u32 {
                self.wanted = self.wanted.saturating_add(1);
            } else if t == METransformHaveOutput.0 as u32 {
                self.ready = self.ready.saturating_add(1);
                while self.ready > 0 {
                    self.ready -= 1;
                    if let Some(p) = self.output(api)? {
                        out.push(p);
                    }
                }
            }
        }
        Ok(())
    }

    /// The type of the next event, `None` when there is none (and `wait` is false). Waiting polls
    /// until the deadline (counted from `start`), so a stuck driver errors instead of hanging.
    fn next_event(&self, start: Instant, wait: bool) -> Result<Option<u32>, String> {
        let Some(events) = &self.events else { return Ok(None) };
        loop {
            // SAFETY: plain COM calls on live interfaces.
            match unsafe { events.GetEvent(MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS(MF_EVENT_FLAG_NO_WAIT.0)) } {
                Ok(ev) => {
                    // SAFETY: plain COM calls on a live interface.
                    let (t, status) = unsafe { (ev.GetType().unwrap_or(0), ev.GetStatus().unwrap_or_default()) };
                    if status.is_err() {
                        return Err(format!("{}: the encoder reported an error ({status:?})", self.name));
                    }
                    return Ok(Some(t));
                }
                Err(e) if e.code() == MF_E_NO_EVENTS_AVAILABLE => {
                    if !wait {
                        return Ok(None);
                    }
                    if start.elapsed() > DEADLINE {
                        return Err(format!("{}: the encoder stopped answering", self.name));
                    }
                    std::thread::sleep(Duration::from_micros(500));
                }
                Err(e) => return Err(format!("{}: GetEvent failed: {e}", self.name)),
            }
        }
    }

    /// Synchronous MFTs: every packet the MFT has ready.
    fn drain_sync(&mut self, api: &MfApi, out: &mut Vec<Packet>) -> Result<(), String> {
        for _ in 0..MAX_OUTPUTS_PER_CALL {
            match self.output(api)? {
                Some(p) => out.push(p),
                None => return Ok(()),
            }
        }
        Ok(())
    }

    /// One packet, or `None` when the MFT needs more input.
    fn output(&mut self, api: &MfApi) -> Result<Option<Packet>, String> {
        for _ in 0..4 {
            let sample = if self.provides_samples { None } else { Some(api.empty_sample(self.out_size)?.0) };
            let mut buf = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: ManuallyDrop::new(sample), dwStatus: 0, pEvents: ManuallyDrop::new(None) }];
            let mut status = 0u32;
            // SAFETY: one output buffer descriptor (with our sample, or none when the MFT provides
            // its own) and a live status out-pointer.
            let r = unsafe { self.transform.ProcessOutput(0, &mut buf, &mut status) };
            // SAFETY: the descriptor's fields were initialised above and are taken once; dropping
            // them releases the references they hold.
            let (sample, _events) = unsafe { (ManuallyDrop::take(&mut buf[0].pSample), ManuallyDrop::take(&mut buf[0].pEvents)) };
            match r {
                Ok(()) => {
                    let sample = sample.ok_or_else(|| format!("{}: produced no sample", self.name))?;
                    return self.packet(&sample).map(Some);
                }
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(None),
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => self.renegotiate()?,
                Err(e) => return Err(format!("{}: ProcessOutput failed: {e}", self.name)),
            }
        }
        Err(format!("{}: keeps changing its output format", self.name))
    }

    /// The output type changed (some encoders publish their sequence header that way): take the
    /// first H.264 type it offers.
    fn renegotiate(&self) -> Result<(), String> {
        // SAFETY: plain COM calls on live interfaces.
        unsafe {
            let t = self.transform.GetOutputAvailableType(0, 0).map_err(|e| format!("{}: no output type after a change: {e}", self.name))?;
            self.transform.SetOutputType(0, &t, 0).map_err(|e| format!("{}: refused its own output type: {e}", self.name))
        }
    }

    fn packet(&self, sample: &IMFSample) -> Result<Packet, String> {
        // SAFETY: plain COM calls on live interfaces; `Lock` gives `len` readable bytes until
        // `Unlock`, copied out before it.
        unsafe {
            let key = sample.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) != 0;
            let time = sample.GetSampleTime().unwrap_or(0);
            let buffer = sample.ConvertToContiguousBuffer().map_err(|e| format!("{}: unreadable output: {e}", self.name))?;
            let (mut ptr, mut len) = (std::ptr::null_mut::<u8>(), 0u32);
            buffer.Lock(&mut ptr, None, Some(&mut len)).map_err(|e| format!("{}: cannot read the output: {e}", self.name))?;
            let data = if ptr.is_null() { Vec::new() } else { std::slice::from_raw_parts(ptr, len as usize).to_vec() };
            let _ = buffer.Unlock();
            Ok(Packet { data, key, time })
        }
    }
}
