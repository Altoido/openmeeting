//! User settings (JSON in the app data folder).
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub whisper_model: String,
    pub llm_model: String,
    pub use_vad: bool,
    pub language: String,
    pub auto_transcribe: bool,
    pub auto_summarize: bool,
    pub auto_title: bool,
    pub default_template: String,
    pub recording_source: String,
    pub input_device: String,
    pub llm_idle_minutes: u64,
    pub setup_completed: bool,
    pub consent_dismissed: bool,
    pub check_updates: bool,
    pub last_update_check: f64,
    pub skipped_version: String,
    /// learned transcription speed per Whisper model (audio seconds per second)
    pub speeds: std::collections::HashMap<String, f64>,
}
impl Default for Settings {
    fn default() -> Self {
        Self { whisper_model: String::new(), llm_model: String::new(), use_vad: true, language: "auto".into(), auto_transcribe: true,
               auto_summarize: true, auto_title: true, default_template: "general".into(), recording_source: "microphone".into(),
               input_device: String::new(), llm_idle_minutes: 10, setup_completed: false, consent_dismissed: false,
               check_updates: true, last_update_check: 0.0, skipped_version: String::new(), speeds: Default::default() }
    }
}

fn file() -> PathBuf { crate::paths::app_data().join("settings.json") }
static CURRENT: Lazy<Mutex<Settings>> = Lazy::new(|| Mutex::new(std::fs::read(file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()));

pub fn get() -> Settings { CURRENT.lock().unwrap().clone() }
pub fn set(s: Settings) { let _ = std::fs::write(file(), serde_json::to_vec_pretty(&s).unwrap_or_default()); *CURRENT.lock().unwrap() = s; }

fn pick(kind: &str, selected: &str) -> Option<PathBuf> {
    let dir = crate::modelmgr::dir_for(kind);
    if !selected.is_empty() && dir.join(selected).is_file() { return Some(dir.join(selected)); }
    // fall back to the best installed model (catalog order = quality order)
    let installed = crate::modelmgr::installed(kind);
    for spec in crate::modelmgr::catalog().iter().rev().filter(|s| s.kind == kind) {
        if let Some(p) = installed.iter().find(|p| p.file_name().map(|n| n == spec.file_name.as_str()).unwrap_or(false)) { return Some(p.clone()); }
    }
    installed.into_iter().next()
}
pub fn whisper_path() -> Option<PathBuf> { pick("whisper", &get().whisper_model) }
pub fn llm_path() -> Option<PathBuf> { pick("llm", &get().llm_model) }
pub fn vad_path() -> Option<PathBuf> { if get().use_vad { crate::modelmgr::installed("vad").into_iter().next() } else { None } }
