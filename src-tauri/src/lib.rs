mod ai;
mod audio;
mod db;
mod engines;
mod export;
mod modelmgr;
mod models;
mod paths;
mod processor;
mod recorder;
mod settings;
mod sysinfo;
mod updates;

use db::Db;
use models::*;
use processor::{Kind, Processor};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

pub struct AppState {
    db: Arc<Mutex<Db>>,
    proc_: Arc<Processor>,
    rec: Mutex<Option<recorder::Session>>,
    ask_cancel: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
    downloads: Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>,
}
type R<T> = Result<T, String>;
fn e<E: std::fmt::Display>(x: E) -> String { x.to_string() }

// ---------------------------------------------------------------- library
#[tauri::command]
fn system_info() -> Value {
    let (w, l) = modelmgr::recommended();
    json!({ "info": sysinfo::info(), "recommendedWhisper": w, "recommendedLlm": l, "systemAudio": recorder::system_audio_supported(),
            "engines": { "whisper": engines::engine_path("whisper-cli").is_some(), "llama": engines::engine_path("llama-server").is_some() },
            "dataDir": paths::app_data(), "libraryDir": paths::library(), "modelsDir": paths::models() })
}
#[tauri::command] fn settings_get() -> settings::Settings { settings::get() }
#[tauri::command] fn settings_set(s: settings::Settings) { settings::set(s) }

#[tauri::command]
fn meetings_list(st: State<AppState>) -> Value {
    let d = st.db.lock().unwrap();
    json!({ "meetings": d.meetings(), "collections": d.collections(), "openActions": d.action_items(None).iter().filter(|a| !a.done).count() })
}

#[tauri::command]
fn meeting_detail(st: State<AppState>, id: String) -> R<Value> {
    let d = st.db.lock().unwrap();
    let m = d.meeting(&id).ok_or("Meeting not found")?;
    let audio = if m.audio_file.is_empty() { None } else { Some(paths::meeting_folder(&id).join(&m.audio_file)) };
    Ok(json!({ "meeting": m, "audioPath": audio, "segments": d.segments(&id), "speakers": d.speakers(&id), "summaries": d.summaries(&id),
               "tldr": d.tldr(&id).map(|t| json!({"markdown": t.0, "model": t.1, "createdAt": t.2})), "actions": d.action_items(Some(&id)),
               "highlights": d.highlights(&id), "mindmap": d.mindmap(&id).and_then(|j| serde_json::from_str::<Value>(&j).ok()),
               "chat": d.chat(Some(&id)) }))
}

#[tauri::command] fn meeting_save(st: State<AppState>, m: Meeting) -> R<()> { st.db.lock().unwrap().save_meeting(&m).map_err(e) }
#[tauri::command]
fn meeting_delete(app: AppHandle, st: State<AppState>, id: String) {
    processor::cancel(&app, &st.proc_, &st.db, &id);
    st.db.lock().unwrap().delete_meeting(&id);
}
#[tauri::command] fn search(st: State<AppState>, q: String) -> Vec<SearchHit> { st.db.lock().unwrap().search(&q, None, None, 300, false) }

#[tauri::command]
fn import_files(app: AppHandle, st: State<AppState>, files: Vec<String>) -> R<Vec<String>> {
    let mut ids = vec![];
    for p in files {
        let src = PathBuf::from(&p);
        let id = uuid::Uuid::new_v4().to_string();
        let ext = src.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_else(|| "audio".into());
        let dest = paths::meeting_folder(&id).join(format!("original.{ext}"));
        std::fs::copy(&src, &dest).map_err(|x| format!("{}: {x}", src.display()))?;
        let created = std::fs::metadata(&src).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs_f64()).unwrap_or_else(db::now);
        let s = settings::get();
        let m = Meeting { id: id.clone(), title: src.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_else(|| "Imported audio".into()),
            created_at: created, audio_file: dest.file_name().unwrap().to_string_lossy().into(), original_file_name: src.file_name().map(|x| x.to_string_lossy().into()),
            source: "imported".into(), status: "recorded".into(), language: s.language.clone(), template_id: s.default_template.clone(), ..Default::default() };
        st.db.lock().unwrap().save_meeting(&m).map_err(e)?;
        if s.auto_transcribe { processor::enqueue(app.clone(), st.proc_.clone(), st.db.clone(), id.clone(), Kind::Full); }
        ids.push(id);
    }
    let _ = app.emit("meetings-changed", ());
    Ok(ids)
}

// ---------------------------------------------------------------- recording
#[tauri::command] fn input_devices() -> Vec<String> { recorder::input_devices() }
#[tauri::command]
fn record_start(st: State<AppState>, title: String, source: String, device: Option<String>) -> R<String> {
    let mut rec = st.rec.lock().unwrap();
    if rec.is_some() { return Err("A recording is already in progress.".into()); }
    let id = uuid::Uuid::new_v4().to_string();
    *rec = Some(recorder::start(&id, &title, &source, device).map_err(e)?);
    Ok(id)
}
#[tauri::command] fn record_status(st: State<AppState>) -> recorder::Status { st.rec.lock().unwrap().as_ref().map(|s| s.status()).unwrap_or_default() }
#[tauri::command] fn record_pause(st: State<AppState>, paused: bool) { if let Some(s) = st.rec.lock().unwrap().as_ref() { s.set_paused(paused) } }

fn finalize_recording(app: &AppHandle, st: &AppState, folder: PathBuf, man: recorder::Manifest, notes: String, highlights: Vec<(f64, String)>, recovered: bool) -> R<String> {
    let dest = paths::meeting_folder(&man.meeting_id).join("audio.wav");
    let dur = recorder::finalize(&folder, &man, &dest).map_err(|x| format!("{x}. The raw audio is kept in {}", folder.display()))?;
    let s = settings::get();
    let m = Meeting { id: man.meeting_id.clone(), title: man.title.clone(), created_at: man.started_at, duration: dur, audio_file: "audio.wav".into(),
        source: "recording".into(), status: "recorded".into(), status_detail: if recovered { "Recovered after the app closed unexpectedly.".into() } else { String::new() },
        language: s.language.clone(), template_id: s.default_template.clone(), stereo_sources: man.channels == 2, user_notes: notes, ..Default::default() };
    {
        let d = st.db.lock().unwrap();
        d.save_meeting(&m).map_err(e)?;
        for (t, l) in highlights { d.add_highlight(&m.id, t, &l); }
    }
    if s.auto_transcribe && !recovered { processor::enqueue(app.clone(), st.proc_.clone(), st.db.clone(), m.id.clone(), Kind::Full); }
    let _ = app.emit("meetings-changed", ());
    Ok(m.id)
}

#[tauri::command]
async fn record_stop(app: AppHandle, st: State<'_, AppState>, notes: String, highlights: Vec<(f64, String)>) -> R<String> {
    let session = st.rec.lock().unwrap().take().ok_or("No recording in progress.")?;
    let (folder, man) = tauri::async_runtime::spawn_blocking(move || session.stop()).await.map_err(e)?;
    finalize_recording(&app, &st, folder, man, notes, highlights, false)
}

// ---------------------------------------------------------------- processing
#[tauri::command]
fn process(app: AppHandle, st: State<AppState>, id: String, kind: String, template_id: Option<String>) {
    let k = match kind.as_str() { "transcribe" => {
        if let Some(mut m) = st.db.lock().unwrap().meeting(&id) { m.transcribed_until = 0.0; let _ = st.db.lock().map(|d| d.save_meeting(&m)); }
        Kind::TranscribeOnly }
        "summarize" => Kind::Summarize(template_id.unwrap_or_else(|| "general".into())), _ => Kind::Full };
    processor::enqueue(app, st.proc_.clone(), st.db.clone(), id, k);
}
#[tauri::command] fn process_cancel(app: AppHandle, st: State<AppState>, id: String) { processor::cancel(&app, &st.proc_, &st.db, &id) }
#[tauri::command] fn progress_get(st: State<AppState>) -> processor::Progress { st.proc_.snapshot() }

// ---------------------------------------------------------------- editing
#[tauri::command]
fn segment_update(st: State<AppState>, meeting_id: String, seg_id: i64, text: Option<String>, speaker: Option<String>, set_speaker: bool, revert: bool) {
    let d = st.db.lock().unwrap();
    if revert { d.revert_segment(seg_id) } else { d.update_segment(seg_id, text.as_deref(), if set_speaker { Some(speaker.as_deref()) } else { None }) }
    d.reindex_segments(&meeting_id);
}
#[tauri::command] fn speaker_set(st: State<AppState>, meeting_id: String, key: String, name: String) { let d = st.db.lock().unwrap(); d.set_speaker(&meeting_id, &key, &name); d.reindex_segments(&meeting_id); }
#[tauri::command] fn highlight_add(st: State<AppState>, meeting_id: String, time: f64, label: String) { st.db.lock().unwrap().add_highlight(&meeting_id, time, &label) }
#[tauri::command] fn highlight_delete(st: State<AppState>, id: i64) { st.db.lock().unwrap().delete_highlight(id) }
#[tauri::command] fn summary_update(st: State<AppState>, meeting_id: String, id: i64, markdown: String) { st.db.lock().unwrap().update_summary(&meeting_id, id, &markdown) }
#[tauri::command] fn tldr_save(st: State<AppState>, meeting_id: String, markdown: String, model: String) { st.db.lock().unwrap().save_tldr(&meeting_id, &markdown, &model) }
#[tauri::command] fn action_save(st: State<AppState>, a: ActionItem) { st.db.lock().unwrap().save_action_item(&a, true) }
#[tauri::command] fn action_delete(st: State<AppState>, id: String, meeting_id: String) { st.db.lock().unwrap().delete_action_item(&id, &meeting_id) }
#[tauri::command] fn actions_all(st: State<AppState>) -> Vec<ActionItem> { st.db.lock().unwrap().action_items(None) }
#[tauri::command] fn templates_list(st: State<AppState>) -> Vec<ai::Template> { ai::templates(&st.db.lock().unwrap()) }
#[tauri::command] fn custom_templates(st: State<AppState>) -> Vec<(String, String, String)> { st.db.lock().unwrap().custom_templates() }
#[tauri::command] fn template_save(st: State<AppState>, id: String, name: String, instructions: String) { st.db.lock().unwrap().save_template(&id, &name, &instructions) }
#[tauri::command] fn template_delete(st: State<AppState>, id: String) { st.db.lock().unwrap().delete_template(&id) }
#[tauri::command] fn collection_save(st: State<AppState>, c: Collection) { st.db.lock().unwrap().save_collection(&c) }
#[tauri::command] fn collection_delete(st: State<AppState>, id: String) { st.db.lock().unwrap().delete_collection(&id) }

// ---------------------------------------------------------------- AI on demand
fn llm() -> R<PathBuf> { settings::llm_path().ok_or_else(|| "No AI language model is installed. Open Settings → AI Models.".into()) }

#[tauri::command]
async fn ask(app: AppHandle, st: State<'_, AppState>, question: String, meeting_id: Option<String>) -> R<String> {
    let model = llm()?;
    st.db.lock().unwrap().add_chat(meeting_id.as_deref(), "user", &question);
    let (tx, rx) = tokio::sync::watch::channel(false);
    *st.ask_cancel.lock().unwrap() = Some(tx);
    let a2 = app.clone();
    let res = ai::ask(st.db.clone(), &question, meeting_id.as_deref(), &model, &rx, move |t| { let _ = a2.emit("ask-token", t.to_string()); }).await;
    let answer = match res { Ok(a) => a, Err(x) => return Err(e(x)) };
    let text = if *rx.borrow() { format!("{answer}\n\n_(stopped)_") } else if answer.trim().is_empty() { "_No answer was produced._".into() } else { answer };
    st.db.lock().unwrap().add_chat(meeting_id.as_deref(), "assistant", &text);
    Ok(text)
}
#[tauri::command] fn ask_cancel(st: State<AppState>) { if let Some(t) = st.ask_cancel.lock().unwrap().as_ref() { let _ = t.send(true); } }
#[tauri::command] fn chat_get(st: State<AppState>, meeting_id: Option<String>) -> Vec<ChatMessage> { st.db.lock().unwrap().chat(meeting_id.as_deref()) }
#[tauri::command] fn chat_clear(st: State<AppState>, meeting_id: Option<String>) { st.db.lock().unwrap().clear_chat(meeting_id.as_deref()) }

#[tauri::command]
async fn mindmap_generate(st: State<'_, AppState>, id: String) -> R<Value> {
    let model = llm()?;
    let material = {
        let d = st.db.lock().unwrap();
        d.summaries(&id).first().map(|s| s.markdown.clone()).unwrap_or_else(|| ai::render(&ai::lines(&d.segments(&id), &d.speakers(&id), 45.0)))
    };
    let material: String = material.chars().take(18_000).collect();
    let node = ai::mindmap(&model, &material).await.map_err(e)?;
    let j = serde_json::to_string(&node).map_err(e)?;
    st.db.lock().unwrap().save_mindmap(&id, &j);
    serde_json::from_str(&j).map_err(e)
}

#[tauri::command]
async fn tldr_generate(st: State<'_, AppState>, id: String) -> R<String> {
    let model = llm()?;
    let (notes, m) = {
        let d = st.db.lock().unwrap();
        let s = d.summaries(&id).first().cloned().ok_or("Create the summary first.")?;
        (serde_json::from_str::<Notes>(&s.structured_json).unwrap_or_default(), d.meeting(&id).ok_or("Meeting not found")?)
    };
    let (_, _, ctx) = engines::ensure_llama(&model).await.map_err(e)?;
    let meta = format!("Title: {}\nDuration: {} min", m.title, (m.duration / 60.0).round());
    let md = ai::tldr(&model, &notes, &meta, m.user_notes.trim(), ctx).await.map_err(e)?;
    st.db.lock().unwrap().save_tldr(&id, &md, &model.file_stem().unwrap_or_default().to_string_lossy());
    Ok(md)
}

// ---------------------------------------------------------------- export & files
#[tauri::command]
fn export_file(st: State<AppState>, id: String, content: String, format: String, path: String) -> R<()> {
    let bytes = export::export(&st.db.lock().unwrap(), &id, &content, &format).map_err(e)?;
    std::fs::write(path, bytes).map_err(e)
}
#[tauri::command]
fn export_markdown(st: State<AppState>, id: String, content: String) -> String { export::markdown(&st.db.lock().unwrap(), &id, &content) }
#[tauri::command]
fn export_text(path: String, markdown: String, format: String, title: String) -> R<()> {
    let bytes = match format.as_str() { "docx" => export::docx(&title, "", &markdown).map_err(e)?, "txt" => export::plain_text(&markdown).into_bytes(), _ => markdown.into_bytes() };
    std::fs::write(path, bytes).map_err(e)
}
#[tauri::command]
fn export_audio(st: State<AppState>, id: String, path: String) -> R<()> {
    let m = st.db.lock().unwrap().meeting(&id).ok_or("Meeting not found")?;
    std::fs::copy(paths::meeting_folder(&id).join(m.audio_file), path).map(|_| ()).map_err(e)
}
#[tauri::command]
async fn peaks(st: State<'_, AppState>, id: String) -> R<Vec<f32>> {
    let m = st.db.lock().unwrap().meeting(&id).ok_or("Meeting not found")?;
    let p = paths::meeting_folder(&id).join(m.audio_file);
    tauri::async_runtime::spawn_blocking(move || audio::peaks(&p, 600)).await.map_err(e)
}

#[tauri::command]
fn write_base64(path: String, data: String) -> R<()> {
    use base64::Engine;
    std::fs::write(path, base64::engine::general_purpose::STANDARD.decode(data).map_err(e)?).map_err(e)
}
#[tauri::command] fn print_page(window: tauri::WebviewWindow) -> R<()> { window.print().map_err(e) }

// ---------------------------------------------------------------- models
#[tauri::command]
fn models_state() -> Value {
    let inst = |k: &str| modelmgr::installed(k).iter().map(|p| json!({"file": p.file_name().unwrap().to_string_lossy(), "bytes": std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)})).collect::<Vec<_>>();
    json!({ "catalog": modelmgr::catalog(), "installed": { "whisper": inst("whisper"), "llm": inst("llm"), "vad": inst("vad") },
            "activeWhisper": settings::whisper_path().map(|p| p.file_name().unwrap().to_string_lossy().to_string()),
            "activeLlm": settings::llm_path().map(|p| p.file_name().unwrap().to_string_lossy().to_string()) })
}
#[tauri::command]
async fn model_download(app: AppHandle, st: State<'_, AppState>, id: String, url: Option<String>) -> R<()> {
    let spec = match url {
        Some(u) => {
            if !u.starts_with("https://") || !u.to_lowercase().ends_with(".gguf") { return Err("Enter an https:// link that ends in .gguf".into()); }
            let f = u.rsplit('/').next().unwrap_or("model.gguf").to_string();
            modelmgr::ModelSpec { id: id.clone(), kind: "llm".into(), name: f.clone(), file_name: f, url: u, approx_bytes: 0, sha1: None, license: String::new(), summary: String::new(), context: 0, min_ram_gb: 0.0 }
        }
        None => modelmgr::catalog().into_iter().find(|s| s.id == id).ok_or("Unknown model")?,
    };
    let (tx, rx) = tokio::sync::watch::channel(false);
    st.downloads.lock().unwrap().insert(id.clone(), tx);
    let (a2, i2) = (app.clone(), id.clone());
    let mut last = std::time::Instant::now();
    let res = modelmgr::download(&spec, rx, move |got, total| {
        if last.elapsed().as_millis() > 250 { last = std::time::Instant::now(); let _ = a2.emit("download-progress", json!({"id": i2, "received": got, "total": total})); }
    }).await;
    st.downloads.lock().unwrap().remove(&id);
    let _ = app.emit("download-progress", json!({"id": id, "done": true}));
    res.map(|_| ()).map_err(e)
}
#[tauri::command] fn model_cancel(st: State<AppState>, id: String) { if let Some(t) = st.downloads.lock().unwrap().get(&id) { let _ = t.send(true); } }
#[tauri::command]
async fn model_delete(kind: String, file: String) -> R<()> {
    if kind == "llm" { engines::LLAMA.lock().await.stop(); }
    std::fs::remove_file(modelmgr::dir_for(&kind).join(file)).map_err(e)
}
#[tauri::command] fn model_import(path: String, kind: String) -> R<String> { modelmgr::import(std::path::Path::new(&path), &kind).map(|p| p.display().to_string()).map_err(e) }
#[tauri::command]
async fn llm_test() -> R<String> {
    let model = llm()?;
    let t = std::time::Instant::now();
    let out = engines::complete(&model, &[("user".into(), "Reply with one short sentence confirming you are ready to take meeting notes.".into())], 60, 0.2, None).await.map_err(e)?;
    Ok(format!("✓ {:.1} s — “{}”", t.elapsed().as_secs_f64(), out.chars().take(120).collect::<String>()))
}

#[tauri::command]
async fn check_update(app: AppHandle, manual: bool) -> R<Option<updates::Update>> {
    let mut s = settings::get();
    if !manual && (!s.check_updates || db::now() - s.last_update_check < 12.0 * 3600.0) { return Ok(None); }
    let current = app.package_info().version.to_string();
    let found = updates::check(&current).await.map_err(e)?;
    s.last_update_check = db::now();
    settings::set(s.clone());
    Ok(found.filter(|u| manual || u.version != s.skipped_version))
}
#[tauri::command] fn app_version(app: AppHandle) -> String { app.package_info().version.to_string() }

// ---------------------------------------------------------------- app
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let database = Db::open(&paths::database()).expect("could not open the meeting database");
    database.mark_interrupted();
    let state = AppState { db: Arc::new(Mutex::new(database)), proc_: Arc::new(Processor::new()), rec: Mutex::new(None),
                           ask_cancel: Mutex::new(None), downloads: Mutex::new(HashMap::new()) };
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .setup(|app| {
            if let Ok(dir) = app.path().resource_dir() {
                let p = dir.join("prompts");
                if p.join("system.txt").is_file() { let _ = ai::PROMPTS_DIR.set(p); }
            }
            // Recover recordings left unfinished by a crash or forced quit.
            let handle = app.handle().clone();
            let st: State<AppState> = app.state();
            for (folder, man) in recorder::orphans() {
                match finalize_recording(&handle, &st, folder, man, String::new(), vec![], true) {
                    Ok(id) => paths::log(&format!("recovered recording {id}")), Err(x) => paths::log(&format!("recovery failed: {x}")) }
            }
            // Unload the AI model after it has been idle for a while.
            tauri::async_runtime::spawn(async { loop { tokio::time::sleep(std::time::Duration::from_secs(60)).await; engines::unload_if_idle(settings::get().llm_idle_minutes.max(1)).await; } });
            Ok(())
        })
        .on_window_event(|w, ev| if let tauri::WindowEvent::Destroyed = ev {
            let st: State<AppState> = w.state();
            if let Some(s) = st.rec.lock().unwrap().take() { let _ = s.stop(); }   // chunks are recovered on next launch
            tauri::async_runtime::block_on(async { engines::LLAMA.lock().await.stop(); });
        })
        .invoke_handler(tauri::generate_handler![
            system_info, settings_get, settings_set, meetings_list, meeting_detail, meeting_save, meeting_delete, search, import_files,
            input_devices, record_start, record_status, record_pause, record_stop, process, process_cancel, progress_get,
            segment_update, speaker_set, highlight_add, highlight_delete, summary_update, tldr_save, action_save, action_delete, actions_all,
            templates_list, custom_templates, template_save, template_delete, collection_save, collection_delete,
            ask, ask_cancel, chat_get, chat_clear, mindmap_generate, tldr_generate, export_file, export_markdown, export_text, export_audio, peaks,
            print_page, write_base64, check_update, app_version, models_state, model_download, model_cancel, model_delete, model_import, llm_test
        ])
        .run(tauri::generate_context!())
        .expect("error while running OpenMeeting Notes");
}
