//! Processing-time estimates. Starts from typical speeds for this kind of computer and model, then
//! learns this computer's real speed from every transcription pass it finishes.
use crate::settings;

/// Audio seconds transcribed per wall-clock second (higher = faster).
fn default_speed(model_file: &str) -> f64 {
    let gpu = crate::engines::gpu_available();
    let f = model_file.to_lowercase();
    let (gpu_s, cpu_s) = if f.contains("large") { (10.0, 0.6) } else if f.contains("small") { (20.0, 1.6) } else if f.contains("base") { (40.0, 4.0) } else { (12.0, 1.0) };
    if gpu { gpu_s } else { cpu_s * (crate::engines::threads() as f64 / 6.0).clamp(0.5, 2.0) }
}

pub fn transcribe_speed() -> f64 {
    let model = settings::whisper_path().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_default();
    settings::get().speeds.get(&model).copied().unwrap_or_else(|| default_speed(&model))
}

/// Called after each finished pass; keeps a smoothed average per model.
pub fn record_transcribe(audio_secs: f64, wall_secs: f64) {
    if audio_secs < 30.0 || wall_secs <= 0.0 { return; }
    let Some(model) = settings::whisper_path().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())) else { return };
    let measured = audio_secs / wall_secs;
    let mut s = settings::get();
    let v = s.speeds.get(&model).map(|old| old * 0.6 + measured * 0.4).unwrap_or(measured);
    s.speeds.insert(model, v);
    settings::set(s);
}

/// Rough minutes for the AI summary + TL;DR of a meeting this long.
pub fn summary_secs(audio_secs: f64) -> f64 {
    let hours = audio_secs / 3600.0;
    if crate::engines::gpu_available() { 60.0 + hours * 120.0 } else { 120.0 + hours * 480.0 }
}

/// Total seconds to process `audio_secs` of audio that still needs `remaining_audio` transcribed.
pub fn total_secs(audio_secs: f64, remaining_audio: f64, summarize: bool) -> f64 {
    remaining_audio / transcribe_speed() + if summarize { summary_secs(audio_secs) } else { 0.0 }
}
