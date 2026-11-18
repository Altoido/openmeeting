//! Record → Transcribe → Summarize pipeline, one meeting at a time, with progress events,
//! cancellation, and checkpoints (10-minute passes) so long meetings resume instead of restarting.
use crate::{ai, audio, db::Db, engines, settings};
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

pub const PASS_SECONDS: f64 = 600.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind { Full, TranscribeOnly, Summarize(String) }

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Progress { pub meeting_id: Option<String>, pub stage: String, pub fraction: f64, pub queued: Vec<String> }

pub struct Processor {
    pub queue: Mutex<VecDeque<(String, Kind)>>,
    pub current: Mutex<Progress>,
    cancel: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
    running: Mutex<bool>,
}

impl Processor {
    pub fn new() -> Self { Self { queue: Mutex::new(VecDeque::new()), current: Mutex::new(Progress::default()), cancel: Mutex::new(None), running: Mutex::new(false) } }

    pub fn snapshot(&self) -> Progress {
        let mut p = self.current.lock().unwrap().clone();
        p.queued = self.queue.lock().unwrap().iter().map(|(id, _)| id.clone()).collect();
        p
    }
    pub fn is_queued(&self, id: &str) -> bool {
        self.current.lock().unwrap().meeting_id.as_deref() == Some(id) || self.queue.lock().unwrap().iter().any(|(m, _)| m == id)
    }
}

fn emit_changed(app: &AppHandle) { let _ = app.emit("meetings-changed", ()); }

pub fn enqueue(app: AppHandle, proc_: Arc<Processor>, db: Arc<Mutex<Db>>, id: String, kind: Kind) {
    if proc_.is_queued(&id) { return; }
    if !matches!(kind, Kind::Summarize(_)) { db.lock().unwrap().set_status(&id, "queued", "Waiting to start…", 0.0); }
    proc_.queue.lock().unwrap().push_back((id, kind));
    emit_changed(&app);
    pump(app, proc_, db);
}

pub fn cancel(app: &AppHandle, proc_: &Arc<Processor>, db: &Arc<Mutex<Db>>, id: &str) {
    let mut q = proc_.queue.lock().unwrap();
    if let Some(i) = q.iter().position(|(m, _)| m == id) { q.remove(i); restore(db, id, "Cancelled."); }
    drop(q);
    if proc_.current.lock().unwrap().meeting_id.as_deref() == Some(id) {
        if let Some(tx) = proc_.cancel.lock().unwrap().as_ref() { let _ = tx.send(true); }
    }
    emit_changed(app);
}

fn restore(db: &Arc<Mutex<Db>>, id: &str, note: &str) {
    let d = db.lock().unwrap();
    let status = if !d.summaries(id).is_empty() { "ready" } else if !d.segments(id).is_empty() { "transcribed" } else { "recorded" };
    d.set_status(id, status, note, 0.0);
}

fn pump(app: AppHandle, proc_: Arc<Processor>, db: Arc<Mutex<Db>>) {
    {
        let mut r = proc_.running.lock().unwrap();
        if *r { return; }
        *r = true;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            let job = proc_.queue.lock().unwrap().pop_front();
            let Some((id, kind)) = job else { break };
            let (tx, rx) = tokio::sync::watch::channel(false);
            *proc_.cancel.lock().unwrap() = Some(tx);
            *proc_.current.lock().unwrap() = Progress { meeting_id: Some(id.clone()), stage: "Starting…".into(), fraction: 0.0, queued: vec![] };
            let res = run(&app, &proc_, &db, &id, kind, rx.clone()).await;
            if let Err(e) = res {
                if *rx.borrow() || e.to_string() == "cancelled" { restore(&db, &id, "Cancelled. Your recording is safe — you can resume any time."); }
                else { crate::paths::log(&format!("processing {id} failed: {e:#}")); db.lock().unwrap().set_status(&id, "failed", &e.to_string(), 0.0); }
            }
            *proc_.current.lock().unwrap() = Progress::default();
            emit_changed(&app);
        }
        *proc_.running.lock().unwrap() = false;
    });
}

fn set(app: &AppHandle, proc_: &Arc<Processor>, db: &Arc<Mutex<Db>>, id: &str, status: &str, stage: &str, f: f64) {
    let mut c = proc_.current.lock().unwrap();
    let changed = c.stage != stage || (f - c.fraction).abs() >= 0.01;
    c.stage = stage.into(); c.fraction = f;
    drop(c);
    if changed {
        db.lock().unwrap().set_status(id, status, stage, f);
        let _ = app.emit("progress", proc_.snapshot());
    }
}

async fn run(app: &AppHandle, p: &Arc<Processor>, db: &Arc<Mutex<Db>>, id: &str, kind: Kind, cancel: tokio::sync::watch::Receiver<bool>) -> Result<()> {
    let m = db.lock().unwrap().meeting(id).ok_or_else(|| anyhow!("meeting not found"))?;
    match kind {
        Kind::Full => {
            let complete = m.transcribed_until > 0.0 && m.duration > 0.0 && m.transcribed_until >= m.duration - 1.0 && !db.lock().unwrap().segments(id).is_empty();
            if !complete { transcribe(app, p, db, id, cancel.clone()).await?; }
            if settings::get().auto_summarize && settings::llm_path().is_some() { summarize(app, p, db, id, &m.template_id).await?; }
            else { db.lock().unwrap().set_status(id, "transcribed", if settings::llm_path().is_none() { "Transcript ready. Install an AI language model to generate summaries." } else { "" }, 1.0); }
        }
        Kind::TranscribeOnly => { transcribe(app, p, db, id, cancel.clone()).await?; restore(db, id, ""); }
        Kind::Summarize(t) => summarize(app, p, db, id, &t).await?,
    }
    Ok(())
}

async fn transcribe(app: &AppHandle, p: &Arc<Processor>, db: &Arc<Mutex<Db>>, id: &str, cancel: tokio::sync::watch::Receiver<bool>) -> Result<()> {
    let mut m = db.lock().unwrap().meeting(id).ok_or_else(|| anyhow!("meeting not found"))?;
    let audio_path = crate::paths::meeting_folder(id).join(&m.audio_file);
    if m.audio_file.is_empty() || !audio_path.is_file() { return Err(anyhow!("The audio file for this meeting is missing.")); }
    let model = settings::whisper_path().ok_or_else(|| anyhow!("No speech-to-text model is installed. Open Settings → AI Models."))?;
    set(app, p, db, id, "preparing", "Preparing audio…", 0.0);
    let wav = crate::paths::meeting_folder(id).join("transcribe-16k.wav");
    let (a2, w2, stereo, c2) = (audio_path.clone(), wav.clone(), m.stereo_sources, cancel.clone());
    let (app2, p2, db2, id2) = (app.clone(), p.clone(), db.clone(), id.to_string());
    let prepared = tokio::task::spawn_blocking(move || audio::prepare(&a2, &w2, stereo, &|| *c2.borrow(),
        |f| set(&app2, &p2, &db2, &id2, "preparing", "Preparing audio…", f * 0.05))).await??;
    if m.duration <= 0.0 { m.duration = prepared.duration; db.lock().unwrap().save_meeting(&m)?; }
    let (samples, rate) = audio::read_wav_mono(&wav)?;
    let dur = prepared.duration;
    let mut bounds = vec![0.0];
    bounds.extend(audio::split_points(&samples, rate, PASS_SECONDS));
    bounds.push(dur);
    let mut resume = m.transcribed_until;
    if resume >= dur - 0.5 || resume < 0.0 { resume = 0.0; }
    { let d = db.lock().unwrap(); d.clear_segments_from(id, resume); }
    let mut prev: String = db.lock().unwrap().segments(id).iter().rev().take(3).rev().map(|s| s.text.clone()).collect::<Vec<_>>().join(" ");
    let lang = if m.language.is_empty() { settings::get().language } else { m.language.clone() };
    let vad = settings::vad_path();
    for i in 0..bounds.len() - 1 {
        let (start, end) = (bounds[i], bounds[i + 1]);
        if end <= resume + 0.01 { continue; }
        if *cancel.borrow() { return Err(anyhow!("cancelled")); }
        let label = if bounds.len() > 2 { format!("Transcribing (part {} of {})…", i + 1, bounds.len() - 1) } else { "Transcribing…".into() };
        let base = 0.05 + 0.95 * start / dur.max(1.0);
        let span = 0.95 * (end - start) / dur.max(1.0);
        set(app, p, db, id, "transcribing", &label, base);
        // Each pass is its own file: whisper.cpp applies offsets after silence removal, so this keeps timestamps exact.
        let pass = crate::paths::meeting_folder(id).join(format!("pass-{i}.wav"));
        let a = (start * rate as f64) as usize; let b = ((end * rate as f64) as usize).min(samples.len());
        audio::write_wav(&pass, &samples[a..b], rate)?;
        let (app2, p2, db2, id2, l2) = (app.clone(), p.clone(), db.clone(), id.to_string(), label.clone());
        let raw = engines::transcribe(&pass, &model, &lang, Some(&prev), vad.as_deref(), &cancel,
            move |f| set(&app2, &p2, &db2, &id2, "transcribing", &l2, base + span * f)).await;
        let _ = std::fs::remove_file(&pass);
        let raw = raw?;
        let segs: Vec<(f64, f64, String, Option<String>)> = raw.into_iter().filter_map(|r| {
            let s = (r.start + start).max(start);
            let e = (r.end + start).min(end + 2.0).max(s);
            if s >= end + 1.0 { return None; }
            let speaker = if m.stereo_sources { speaker_for(s, e, &prepared) } else { None };
            Some((s, e, r.text, speaker))
        }).collect();
        { let d = db.lock().unwrap(); d.append_segments(id, &segs)?; d.set_transcribed_until(id, end); }
        prev = segs.iter().rev().take(3).rev().map(|s| s.2.clone()).collect::<Vec<_>>().join(" ");
        emit_changed(app);
    }
    let _ = std::fs::remove_file(&wav);
    let d = db.lock().unwrap();
    d.reindex_segments(id);
    if m.stereo_sources {
        let names = d.speakers(id);
        if !names.contains_key("ME") { d.set_speaker(id, "ME", "Me"); }
        if !names.contains_key("REMOTE") { d.set_speaker(id, "REMOTE", "Remote participants"); }
    }
    d.set_status(id, "transcribed", "", 1.0);
    Ok(())
}

/// Two-source recordings: microphone on the left, computer audio on the right → who was talking?
fn speaker_for(start: f64, end: f64, p: &audio::Prepared) -> Option<String> {
    let a = (start * 10.0) as usize; let b = ((end * 10.0) as usize + 1).min(p.left_energy.len()).min(p.right_energy.len());
    if a >= b { return None; }
    let l: f32 = p.left_energy[a..b].iter().sum(); let r: f32 = p.right_energy[a..b].iter().sum();
    if l < 0.0001 && r < 0.0001 { return None; }
    Some(if r > l * 1.2 { "REMOTE".into() } else { "ME".into() })
}

async fn summarize(app: &AppHandle, p: &Arc<Processor>, db: &Arc<Mutex<Db>>, id: &str, template_id: &str) -> Result<()> {
    let model = settings::llm_path().ok_or_else(|| anyhow!("No AI language model is installed. Open Settings → AI Models."))?;
    let (tpl, meeting) = { let d = db.lock().unwrap(); (ai::template(&d, template_id), d.meeting(id).ok_or_else(|| anyhow!("meeting not found"))?) };
    let want_title = settings::get().auto_title && !meeting.title_user_set;
    let (app2, p2, db2, id2) = (app.clone(), p.clone(), db.clone(), id.to_string());
    let r = ai::summarize(db.clone(), id, &tpl, want_title, &model, move |s, f| set(&app2, &p2, &db2, &id2, "summarizing", s, f)).await?;
    let d = db.lock().unwrap();
    d.add_summary(id, &r.summary);
    if let Some(t) = &r.tldr { if !t.trim().is_empty() { d.save_tldr(id, t, &r.summary.model_name); } }
    d.replace_generated_actions(id, &ai::action_items(&r.notes, id));
    if let Some(mut m) = d.meeting(id) {
        if let (Some(t), false) = (&r.title, m.title_user_set) {
            let date = chrono::DateTime::from_timestamp(m.created_at as i64, 0).map(|x| x.with_timezone(&chrono::Local).format("%B %-d").to_string()).unwrap_or_default();
            m.title = format!("{t} — {date}");
        }
        m.template_id = tpl.id.clone();
        d.save_meeting(&m)?;
    }
    d.set_status(id, "ready", if r.parts > 1 { "Summarized in several parts." } else { "" }, 1.0);
    Ok(())
}
