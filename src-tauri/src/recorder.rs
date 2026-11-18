//! Crash-safe recording: microphone and/or computer audio → 1-minute WAV chunks (24 kHz, 16-bit),
//! merged into one file on stop. Two-source recordings are stereo: left = microphone, right = computer.
//! Computer audio: Windows uses WASAPI loopback (no permission needed). macOS system-audio capture
//! (ScreenCaptureKit) is not in this build yet — use the microphone, or the original macOS app.
use anyhow::{anyhow, bail, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const REC_RATE: u32 = 24_000;
const CHUNK_SECONDS: u64 = 60;
const LATENCY: f64 = 0.35;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Manifest { pub meeting_id: String, pub title: String, pub started_at: f64, pub channels: u16, pub source: String }

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status { pub active: bool, pub paused: bool, pub elapsed: f64, pub mic_level: f32, pub system_level: f32, pub warning: Option<String>, pub meeting_id: Option<String> }

pub fn system_audio_supported() -> bool { cfg!(windows) }

pub fn input_devices() -> Vec<String> {
    cpal::default_host().input_devices().map(|it| it.filter_map(|d| d.name().ok()).collect()).unwrap_or_default()
}

struct Shared {
    mic: Mutex<VecDeque<f32>>, sys: Mutex<VecDeque<f32>>,
    stop: AtomicBool, paused: AtomicBool,
    mic_peak: AtomicU32, sys_peak: AtomicU32, frames: AtomicU64,
    warning: Mutex<Option<String>>,
}

pub struct Session { shared: Arc<Shared>, pub manifest: Manifest, pub folder: PathBuf, threads: Vec<std::thread::JoinHandle<()>> }

fn bump_peak(a: &AtomicU32, v: f32) { let cur = f32::from_bits(a.load(Ordering::Relaxed)); if v > cur { a.store(v.to_bits(), Ordering::Relaxed); } }
fn meter(rms: f32) -> f32 { if rms <= 0.0 { 0.0 } else { ((20.0 * rms.log10() + 60.0) / 60.0).clamp(0.0, 1.0) } }

/// Builds a capture stream that downmixes to mono, resamples to REC_RATE and pushes into `fifo`.
fn build_stream(device: &cpal::Device, loopback: bool, shared: Arc<Shared>, is_mic: bool) -> Result<cpal::Stream> {
    let config = if loopback { device.default_output_config()? } else { device.default_input_config()? };
    let ch = config.channels() as usize;
    let rate = config.sample_rate().0 as f64;
    let mut rs = crate::audio::Resampler::new(rate, REC_RATE as f64);
    let err = |e| crate::paths::log(&format!("audio stream error: {e}"));
    macro_rules! stream_for {
        ($t:ty, $conv:expr) => {{
            let sh = shared.clone();
            device.build_input_stream(&config.config(), move |data: &[$t], _| {
                let mono: Vec<f32> = data.chunks(ch).map(|f| f.iter().map(|s| $conv(*s)).sum::<f32>() / ch as f32).collect();
                let out = rs.process(&mono);
                let rms = (out.iter().map(|v| v * v).sum::<f32>() / out.len().max(1) as f32).sqrt();
                bump_peak(if is_mic { &sh.mic_peak } else { &sh.sys_peak }, rms);
                if !sh.paused.load(Ordering::Relaxed) { (if is_mic { &sh.mic } else { &sh.sys }).lock().unwrap().extend(out); }
            }, err, None)?
        }};
    }
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => stream_for!(f32, |s: f32| s),
        cpal::SampleFormat::I16 => stream_for!(i16, |s: i16| s as f32 / 32768.0),
        cpal::SampleFormat::U16 => stream_for!(u16, |s: u16| (s as f32 - 32768.0) / 32768.0),
        cpal::SampleFormat::I32 => stream_for!(i32, |s: i32| s as f32 / 2_147_483_648.0),
        f => bail!("Unsupported audio sample format {f:?}"),
    };
    stream.play()?;
    Ok(stream)
}

fn chunk_writer(folder: &Path, idx: u32, channels: u16) -> Result<hound::WavWriter<std::io::BufWriter<std::fs::File>>> {
    Ok(hound::WavWriter::create(folder.join(format!("chunk-{idx:05}.wav")),
        hound::WavSpec { channels, sample_rate: REC_RATE, bits_per_sample: 16, sample_format: hound::SampleFormat::Int })?)
}

pub fn start(meeting_id: &str, title: &str, source: &str, device_name: Option<String>) -> Result<Session> {
    if crate::sysinfo::free_disk_gb() < 1.0 { bail!("There is less than 1 GB of free disk space. Free up space before recording."); }
    let use_mic = source != "system";
    let use_sys = source != "microphone";
    if use_sys && !system_audio_supported() { bail!("Recording computer audio isn't available on this system in this version yet. Choose “Microphone only”."); }
    let channels: u16 = if use_mic && use_sys { 2 } else { 1 };
    let folder = crate::paths::in_progress().join(meeting_id);
    std::fs::create_dir_all(&folder)?;
    let manifest = Manifest { meeting_id: meeting_id.into(), title: title.into(), started_at: crate::db::now(), channels, source: source.into() };
    std::fs::write(folder.join("manifest.json"), serde_json::to_vec(&manifest)?)?;
    let shared = Arc::new(Shared { mic: Mutex::new(VecDeque::new()), sys: Mutex::new(VecDeque::new()), stop: AtomicBool::new(false), paused: AtomicBool::new(false),
        mic_peak: AtomicU32::new(0), sys_peak: AtomicU32::new(0), frames: AtomicU64::new(0), warning: Mutex::new(None) });

    // Capture thread owns the (non-Send) cpal streams for the whole session.
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
    let sh = shared.clone();
    let capture = std::thread::spawn(move || {
        let host = cpal::default_host();
        let mut streams = vec![];
        let res = (|| -> Result<()> {
            if use_mic {
                let dev = match &device_name {
                    Some(n) if !n.is_empty() => host.input_devices()?.find(|d| d.name().map(|x| &x == n).unwrap_or(false)),
                    _ => None,
                }.or_else(|| host.default_input_device()).ok_or_else(|| anyhow!("No microphone was found."))?;
                streams.push(build_stream(&dev, false, sh.clone(), true).map_err(|e| anyhow!("Could not open the microphone ({e}). Check that microphone access is allowed for OpenMeeting Notes in your system privacy settings."))?);
            }
            if use_sys {
                let dev = host.default_output_device().ok_or_else(|| anyhow!("No speaker/output device was found for computer audio."))?;
                streams.push(build_stream(&dev, true, sh.clone(), false)?);
            }
            Ok(())
        })();
        let ok = res.is_ok();
        let _ = ready_tx.send(res);
        if !ok { return; }
        while !sh.stop.load(Ordering::Relaxed) { std::thread::sleep(Duration::from_millis(50)); }
        drop(streams);
    });
    ready_rx.recv().map_err(|_| anyhow!("audio thread failed"))?.map_err(|e| { let _ = std::fs::remove_dir_all(&folder); e })?;

    // Writer thread: wall-clock aligned, so both sources stay in sync; pads silence if a source stalls.
    let sh = shared.clone();
    let f2 = folder.clone();
    let writer = std::thread::spawn(move || {
        let mut idx = 1u32;
        let mut w = match chunk_writer(&f2, idx, channels) { Ok(w) => w, Err(e) => { *sh.warning.lock().unwrap() = Some(format!("Writing audio failed: {e}")); return; } };
        let (mut in_chunk, mut written) = (0u64, 0u64);
        let mut t0 = Instant::now();
        let mut base = 0u64;
        let mut was_paused = false;
        loop {
            std::thread::sleep(Duration::from_millis(100));
            let stopping = sh.stop.load(Ordering::Relaxed);
            let paused = sh.paused.load(Ordering::Relaxed);
            if paused { was_paused = true; if !stopping { continue; } }
            if was_paused && !paused { t0 = Instant::now(); base = written; sh.mic.lock().unwrap().clear(); sh.sys.lock().unwrap().clear(); was_paused = false; }
            let mut need = if stopping && !paused {
                (if use_mic { sh.mic.lock().unwrap().len() } else { 0 }).max(if use_sys { sh.sys.lock().unwrap().len() } else { 0 }) as u64
            } else if stopping { 0 } else {
                let t = t0.elapsed().as_secs_f64() - LATENCY;
                if t <= 0.0 { continue; }
                (base + (t * REC_RATE as f64) as u64).saturating_sub(written)
            };
            need = need.min(REC_RATE as u64 * 5);
            let take = |q: &Mutex<VecDeque<f32>>, n: usize| -> Vec<f32> {
                let mut q = q.lock().unwrap();
                let k = n.min(q.len());
                let mut v: Vec<f32> = q.drain(..k).collect();
                v.resize(n, 0.0);
                let excess = q.len().saturating_sub(REC_RATE as usize);    // bound drift to ~1 s
                q.drain(..excess);
                v
            };
            let n = need as usize;
            let mic = if use_mic { take(&sh.mic, n) } else { vec![] };
            let sys = if use_sys { take(&sh.sys, n) } else { vec![] };
            for i in 0..n {
                if in_chunk >= CHUNK_SECONDS * REC_RATE as u64 {
                    let _ = w.finalize();
                    idx += 1; in_chunk = 0;
                    w = match chunk_writer(&f2, idx, channels) { Ok(w) => w, Err(e) => { *sh.warning.lock().unwrap() = Some(format!("Writing audio failed: {e}")); return; } };
                }
                let l = if use_mic { mic[i] } else { sys[i] };
                let _ = w.write_sample((l.clamp(-1.0, 1.0) * 32767.0) as i16);
                if channels == 2 { let _ = w.write_sample((sys[i].clamp(-1.0, 1.0) * 32767.0) as i16); }
                in_chunk += 1; written += 1;
            }
            sh.frames.store(written, Ordering::Relaxed);
            if stopping { let _ = w.finalize(); return; }
        }
    });
    paths_log_start(&manifest);
    Ok(Session { shared, manifest, folder, threads: vec![capture, writer] })
}

fn paths_log_start(m: &Manifest) { crate::paths::log(&format!("Recording started ({}) {}", m.source, m.meeting_id)); }

impl Session {
    pub fn status(&self) -> Status {
        let mic = f32::from_bits(self.shared.mic_peak.swap(0, Ordering::Relaxed));
        let sys = f32::from_bits(self.shared.sys_peak.swap(0, Ordering::Relaxed));
        Status { active: true, paused: self.shared.paused.load(Ordering::Relaxed), elapsed: self.shared.frames.load(Ordering::Relaxed) as f64 / REC_RATE as f64,
                 mic_level: meter(mic), system_level: meter(sys), warning: self.shared.warning.lock().unwrap().clone(), meeting_id: Some(self.manifest.meeting_id.clone()) }
    }
    pub fn set_paused(&self, p: bool) { self.shared.paused.store(p, Ordering::Relaxed); }
    pub fn stop(mut self) -> (PathBuf, Manifest) {
        self.shared.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) { let _ = t.join(); }
        (self.folder, self.manifest)
    }
}

/// Merges chunks into one WAV in the meeting folder. Chunks whose header was never finalized
/// (app crash, power loss) are recovered from their raw PCM data.
pub fn finalize(folder: &Path, manifest: &Manifest, dest: &Path) -> Result<f64> {
    let mut chunks: Vec<PathBuf> = std::fs::read_dir(folder)?.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "wav").unwrap_or(false)).collect();
    chunks.sort();
    if chunks.is_empty() { bail!("No audio was captured."); }
    let ch = manifest.channels;
    let mut out = hound::WavWriter::create(dest, hound::WavSpec { channels: ch, sample_rate: REC_RATE, bits_per_sample: 16, sample_format: hound::SampleFormat::Int })?;
    let mut frames = 0u64;
    for c in &chunks {
        let samples: Vec<i16> = match hound::WavReader::open(c) {
            Ok(mut r) if r.len() > 0 => r.samples::<i16>().flatten().collect(),
            _ => {
                let data = std::fs::read(c).unwrap_or_default();
                if data.len() > 44 { crate::paths::log(&format!("Recovered unfinished chunk {}", c.display())); }
                data.get(44..).unwrap_or(&[]).chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
            }
        };
        let usable = samples.len() - samples.len() % ch as usize;
        for s in &samples[..usable] { out.write_sample(*s)?; }
        frames += (usable / ch as usize) as u64;
    }
    out.finalize()?;
    if frames == 0 { let _ = std::fs::remove_file(dest); bail!("The recording is empty."); }
    let _ = std::fs::remove_dir_all(folder);
    Ok(frames as f64 / REC_RATE as f64)
}

pub fn orphans() -> Vec<(PathBuf, Manifest)> {
    std::fs::read_dir(crate::paths::in_progress()).map(|it| it.flatten().filter_map(|e| {
        let p = e.path();
        let m: Manifest = serde_json::from_slice(&std::fs::read(p.join("manifest.json")).ok()?).ok()?;
        Some((p, m))
    }).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finalize_recovers_unfinished_chunk() {
        let dir = std::env::temp_dir().join(format!("omn-rec-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let m = Manifest { meeting_id: "x".into(), title: "t".into(), started_at: 0.0, channels: 1, source: "microphone".into() };
        let mut w = chunk_writer(&dir, 1, 1).unwrap();
        for i in 0..REC_RATE { w.write_sample((i % 100) as i16).unwrap(); }
        w.finalize().unwrap();
        // simulate a crash: header present, sizes never written
        let mut raw = std::fs::read(dir.join("chunk-00001.wav")).unwrap();
        raw[4..8].copy_from_slice(&0u32.to_le_bytes()); raw[40..44].copy_from_slice(&0u32.to_le_bytes());
        std::fs::write(dir.join("chunk-00002.wav"), &raw).unwrap();
        let dest = std::env::temp_dir().join(format!("omn-final-{}.wav", uuid::Uuid::new_v4()));
        let d = finalize(&dir, &m, &dest).unwrap();
        assert!((d - 2.0).abs() < 0.01, "{d}");
    }
}
