//! Audio decoding (any common audio/video file → 16 kHz mono WAV, all on-device), splitting, waveform.
use anyhow::{anyhow, bail, Result};
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const RATE: u32 = 16_000;

/// Streaming linear-interpolation resampler (continuous across buffers).
pub struct Resampler { in_rate: f64, out_rate: f64, pos: f64, last: f32, has_last: bool }
impl Resampler {
    pub fn new(in_rate: f64, out_rate: f64) -> Self { Self { in_rate, out_rate, pos: 0.0, last: 0.0, has_last: false } }
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if input.is_empty() { return vec![]; }
        if (self.in_rate - self.out_rate).abs() < 0.5 { return input.to_vec(); }
        let step = self.in_rate / self.out_rate;
        let mut src = Vec::with_capacity(input.len() + 1);
        if self.has_last { src.push(self.last); } else { self.pos = 0.0; }
        src.extend_from_slice(input);
        let mut out = Vec::with_capacity((input.len() as f64 / step) as usize + 2);
        let mut p = self.pos;
        while p + 1.0 < src.len() as f64 {
            let i = p as usize; let f = (p - i as f64) as f32;
            out.push(src[i] * (1.0 - f) + src[i + 1] * f);
            p += step;
        }
        self.last = *src.last().unwrap(); self.has_last = true;
        self.pos = p - (src.len() - 1) as f64;
        out
    }
}

pub struct Prepared { pub duration: f64, pub left_energy: Vec<f32>, pub right_energy: Vec<f32> }

/// Decodes MP3, WAV, M4A/AAC, FLAC, OGG, MP4/MOV (audio track) and writes 16 kHz mono 16-bit WAV.
/// For stereo two-source recordings, also measures left/right energy per 100 ms (Me vs Remote).
pub fn prepare(input: &Path, output: &Path, stereo_energy: bool, cancel: &dyn Fn() -> bool, mut progress: impl FnMut(f64)) -> Result<Prepared> {
    let file = std::fs::File::open(input)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = input.extension().and_then(|e| e.to_str()) { hint.with_extension(ext); }
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| anyhow!("The audio could not be read. The file may be damaged or in an unsupported format. ({e})"))?;
    let mut format = probed.format;
    let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL && t.codec_params.sample_rate.is_some())
        .ok_or_else(|| anyhow!("This file does not contain a supported audio track."))?;
    let track_id = track.id;
    let rate = track.codec_params.sample_rate.unwrap_or(48_000) as f64;
    let total = track.codec_params.n_frames.map(|n| n as f64 / rate).unwrap_or(0.0);
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| anyhow!("Unsupported audio codec ({e})."))?;
    let spec = hound::WavSpec { channels: 1, sample_rate: RATE, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(output, spec)?;
    let mut rs = Resampler::new(rate, RATE as f64);
    let (mut written, mut last_rep) = (0usize, 0.0);
    let (mut le, mut re) = (vec![], vec![]);
    let (mut wl, mut wr, mut wn) = (0f32, 0f32, 0usize);
    let win = (rate / 10.0) as usize;
    loop {
        if cancel() { bail!("cancelled"); }
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(symphonia::core::errors::Error::ResetRequired) => break,
            Err(e) => { if written > 0 { break } else { bail!("The audio could not be read ({e}).") } }
        };
        if packet.track_id() != track_id { continue; }
        let decoded = match decoder.decode(&packet) { Ok(d) => d, Err(symphonia::core::errors::Error::DecodeError(_)) => continue, Err(e) => bail!("Decode error: {e}") };
        let ch = decoded.spec().channels.count();
        let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        buf.copy_interleaved_ref(decoded);
        let s = buf.samples();
        let frames = s.len() / ch.max(1);
        let mut mono = Vec::with_capacity(frames);
        for i in 0..frames {
            let mut sum = 0.0;
            for c in 0..ch { sum += s[i * ch + c]; }
            mono.push(sum / ch as f32);
            if stereo_energy && ch >= 2 {
                let (l, r) = (s[i * ch], s[i * ch + 1]);
                wl += l * l; wr += r * r; wn += 1;
                if wn >= win { le.push((wl / wn as f32).sqrt()); re.push((wr / wn as f32).sqrt()); wl = 0.0; wr = 0.0; wn = 0; }
            }
        }
        for v in rs.process(&mono) { w.write_sample((v.clamp(-1.0, 1.0) * 32767.0) as i16)?; written += 1; }
        if total > 0.0 { let f = (written as f64 / RATE as f64 / total).min(1.0); if f - last_rep >= 0.005 { last_rep = f; progress(f); } }
    }
    w.finalize()?;
    if written == 0 { bail!("No audio samples were found in this file."); }
    Ok(Prepared { duration: written as f64 / RATE as f64, left_energy: le, right_energy: re })
}

pub fn read_wav_mono(path: &Path) -> Result<(Vec<f32>, u32)> {
    let mut r = hound::WavReader::open(path)?;
    let spec = r.spec();
    let ch = spec.channels as usize;
    let all: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => r.samples::<i16>().map(|s| s.unwrap_or(0) as f32 / 32768.0).collect(),
        hound::SampleFormat::Float => r.samples::<f32>().map(|s| s.unwrap_or(0.0)).collect(),
    };
    Ok((all.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect(), spec.sample_rate))
}

/// Quiet points near every `chunk` seconds, so long audio is cut between words.
pub fn split_points(samples: &[f32], rate: u32, chunk: f64) -> Vec<f64> {
    let dur = samples.len() as f64 / rate as f64;
    if dur <= chunk * 1.15 { return vec![]; }
    let (mut pts, mut target) = (vec![], chunk);
    let win = (rate as f64 * 0.2) as usize;
    while target < dur - chunk * 0.15 {
        let a = ((target - 8.0).max(0.0) * rate as f64) as usize;
        let b = (((target + 8.0) * rate as f64) as usize).min(samples.len());
        let (mut best, mut best_e) = (target, f32::MAX);
        let mut i = a;
        while i + win <= b {
            let e: f32 = samples[i..i + win].iter().map(|v| v * v).sum();
            if e < best_e { best_e = e; best = (i + win / 2) as f64 / rate as f64; }
            i += win / 2;
        }
        pts.push(best); target = best + chunk;
    }
    pts
}

pub fn write_wav(path: &Path, samples: &[f32], rate: u32) -> Result<()> {
    let mut w = hound::WavWriter::create(path, hound::WavSpec { channels: 1, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int })?;
    for v in samples { w.write_sample((v.clamp(-1.0, 1.0) * 32767.0) as i16)?; }
    w.finalize()?;
    Ok(())
}

/// Downsampled peak envelope (0…1) for the waveform display.
pub fn peaks(path: &Path, count: usize) -> Vec<f32> {
    let cache = path.with_extension("peaks.json");
    if let Ok(s) = std::fs::read_to_string(&cache) { if let Ok(v) = serde_json::from_str::<Vec<f32>>(&s) { if v.len() == count { return v; } } }
    let tmp = crate::paths::cache().join(format!("peaks-{}.wav", uuid::Uuid::new_v4()));
    let ok = prepare(path, &tmp, false, &|| false, |_| {}).is_ok();
    let (s, _) = if ok { read_wav_mono(&tmp).unwrap_or_default() } else { (vec![], RATE) };
    let _ = std::fs::remove_file(&tmp);
    if s.is_empty() { return vec![]; }
    let per = (s.len() / count).max(1);
    let mut p: Vec<f32> = s.chunks(per).take(count).map(|c| c.iter().fold(0f32, |m, v| m.max(v.abs()))).collect();
    let mx = p.iter().cloned().fold(0.0001f32, f32::max);
    p.iter_mut().for_each(|v| *v /= mx);
    p.resize(count, 0.0);
    let _ = std::fs::write(&cache, serde_json::to_string(&p).unwrap_or_default());
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resampler_continuity() {
        let mut r = Resampler::new(44_100.0, 16_000.0);
        let (mut total, mut last, mut jump) = (0, 0f32, 0f32);
        for b in 0..10 {
            let input: Vec<f32> = (0..4410).map(|k| ((b * 4410 + k) as f32 * 2.0 * std::f32::consts::PI * 220.0 / 44_100.0).sin()).collect();
            let o = r.process(&input);
            if total > 0 { jump = jump.max((o[0] - last).abs()); }
            last = *o.last().unwrap(); total += o.len();
        }
        assert!((total as i64 - 16_000).abs() <= 3 && jump < 0.2);
    }
    #[test]
    fn prepare_wav_roundtrip() {
        let dir = std::env::temp_dir();
        let src = dir.join("omn-src.wav");
        let mut w = hound::WavWriter::create(&src, hound::WavSpec { channels: 2, sample_rate: 44_100, bits_per_sample: 16, sample_format: hound::SampleFormat::Int }).unwrap();
        for i in 0..44_100 * 3 { let v = ((i as f32) * 0.05).sin(); w.write_sample((v * 20000.0) as i16).unwrap(); w.write_sample(0i16).unwrap(); }
        w.finalize().unwrap();
        let out = dir.join("omn-out.wav");
        let p = prepare(&src, &out, true, &|| false, |_| {}).unwrap();
        assert!((p.duration - 3.0).abs() < 0.05, "{}", p.duration);
        assert!(p.left_energy[5] > p.right_energy[5]);
        let (s, r) = read_wav_mono(&out).unwrap();
        assert_eq!(r, 16_000); assert!((s.len() as i64 - 48_000).abs() < 20);
    }
}
