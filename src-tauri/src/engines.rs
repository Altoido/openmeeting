//! Bundled local AI engines (whisper-cli, llama-server) run as supervised child processes:
//! a model crash or out-of-memory can't take down the app or a recording, and cancel is instant.
use anyhow::{anyhow, bail, Result};
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

const EXE: &str = if cfg!(windows) { ".exe" } else { "" };

/// Finds an engine next to the app executable (where Tauri installs sidecars), or in OMN_ENGINES_DIR.
pub fn engine_path(name: &str) -> Option<PathBuf> {
    let mut c = vec![];
    if let Ok(d) = std::env::var("OMN_ENGINES_DIR") { c.push(PathBuf::from(d).join(format!("{name}{EXE}"))); }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            c.push(dir.join(format!("{name}{EXE}")));
            c.push(dir.join(format!("../Resources/{name}{EXE}")));
        }
    }
    c.into_iter().find(|p| p.is_file())
}

fn no_window(cmd: &mut Command) {
    #[cfg(windows)]
    { cmd.creation_flags(0x0800_0000); } // CREATE_NO_WINDOW: don't flash a console
    let _ = cmd;
}

pub fn threads() -> usize { std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).saturating_sub(2).clamp(2, 8) }

pub fn gpu_available() -> bool { cfg!(all(target_os = "macos", target_arch = "aarch64")) }

// ---------------------------------------------------------------- whisper
#[derive(Debug, Clone)]
pub struct RawSegment { pub start: f64, pub end: f64, pub text: String }

pub async fn transcribe(wav: &Path, model: &Path, language: &str, prompt: Option<&str>, vad: Option<&Path>,
                        cancel: &tokio::sync::watch::Receiver<bool>, mut progress: impl FnMut(f64)) -> Result<Vec<RawSegment>> {
    let exe = engine_path("whisper-cli").ok_or_else(|| anyhow!("The transcription engine (whisper-cli) is missing from the app."))?;
    if !model.is_file() { bail!("No speech-to-text model is installed. Open Settings → AI Models."); }
    let out = crate::paths::cache().join(format!("whisper-{}", uuid::Uuid::new_v4()));
    let mut args: Vec<String> = vec!["-m".into(), model.display().to_string(), "-f".into(), wav.display().to_string(),
        "-l".into(), language.into(), "-t".into(), threads().to_string(), "-oj".into(), "-of".into(), out.display().to_string(),
        "-pp".into(), "--suppress-nst".into()];
    if let Some(p) = prompt.filter(|p| !p.trim().is_empty()) {
        let tail: String = p.chars().rev().take(400).collect::<Vec<_>>().into_iter().rev().collect();
        args.push("--prompt".into()); args.push(tail);
    }
    if let Some(v) = vad.filter(|v| v.is_file()) { args.push("--vad".into()); args.push("-vm".into()); args.push(v.display().to_string()); }
    let mut cmd = Command::new(exe);
    cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).kill_on_drop(true);
    no_window(&mut cmd);
    let mut child = cmd.spawn()?;
    let stderr = child.stderr.take().unwrap();
    let mut lines = BufReader::new(stderr).lines();
    let mut tail: Vec<String> = vec![];
    let mut cancel = cancel.clone();
    loop {
        tokio::select! {
            line = lines.next_line() => match line? {
                Some(l) => {
                    if let Some(i) = l.find("progress =") {
                        if let Ok(v) = l[i + 10..].trim().trim_end_matches('%').trim().parse::<f64>() { progress((v / 100.0).min(1.0)); }
                    }
                    tail.push(l); if tail.len() > 30 { tail.remove(0); }
                }
                None => break,
            },
            _ = cancel.changed() => { if *cancel.borrow() { let _ = child.kill().await; bail!("cancelled"); } }
        }
    }
    let status = child.wait().await?;
    let json_path = out.with_extension("json");
    if !status.success() { bail!("Transcription failed: {}", tail.iter().rev().take(6).rev().cloned().collect::<Vec<_>>().join("\n")); }
    let data = std::fs::read(&json_path)?;
    let _ = std::fs::remove_file(&json_path);
    parse_whisper_json(&data)
}

pub fn parse_whisper_json(data: &[u8]) -> Result<Vec<RawSegment>> {
    let v: Value = serde_json::from_str(&String::from_utf8_lossy(data))?;
    let items = v["transcription"].as_array().ok_or_else(|| anyhow!("no transcription in output"))?;
    Ok(items.iter().filter_map(|it| {
        let text = it["text"].as_str()?.trim().to_string();
        if text.is_empty() || is_hallucination(&text) { return None; }
        Some(RawSegment { start: it["offsets"]["from"].as_f64()? / 1000.0, end: it["offsets"]["to"].as_f64()? / 1000.0, text })
    }).collect())
}

pub fn is_hallucination(text: &str) -> bool {
    let raw = text.trim().to_lowercase();
    if (raw.starts_with('[') && raw.ends_with(']')) || (raw.starts_with('(') && raw.ends_with(')')) {
        let inner = raw[1..raw.len() - 1].replace('_', " ");
        if ["blank audio", "silence", "music", "applause", "laughter", "inaudible", "no speech", "sound", "noise"].iter().any(|t| inner.contains(t)) { return true; }
    }
    let t: String = raw.chars().filter(|c| c.is_alphanumeric() || c.is_whitespace()).collect();
    ["thank you for watching", "thanks for watching", "please subscribe", "like and subscribe"].contains(&t.trim())
}

// ---------------------------------------------------------------- llama-server
pub struct Llama {
    child: Option<Child>,
    port: u16,
    key: String,
    pub model: Option<PathBuf>,
    pub ctx: usize,
    last_used: Instant,
}

pub static LLAMA: Lazy<Mutex<Llama>> = Lazy::new(|| Mutex::new(Llama { child: None, port: 0, key: String::new(), model: None, ctx: 8192, last_used: Instant::now() }));
static HTTP: Lazy<reqwest::Client> = Lazy::new(|| reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(4 * 3600)).build().unwrap());

pub fn context_for(model: &Path) -> usize {
    let name = model.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let gb = crate::sysinfo::memory_gb();
    if let Some(spec) = crate::modelmgr::catalog().into_iter().find(|s| s.file_name == name && s.kind == "llm") {
        return if gb < 12.0 { spec.context.min(8192) } else { spec.context };
    }
    if gb >= 32.0 { 16384 } else if gb >= 16.0 { 12288 } else { 8192 }
}

impl Llama {
    pub fn running(&mut self) -> bool { matches!(self.child.as_mut().map(|c| c.try_wait()), Some(Ok(None))) }
    pub fn stop(&mut self) { if let Some(mut c) = self.child.take() { let _ = c.start_kill(); } self.model = None; }
}

pub async fn ensure_llama(model: &Path) -> Result<(u16, String, usize)> {
    let mut l = LLAMA.lock().await;
    if l.running() && l.model.as_deref() == Some(model) { l.last_used = Instant::now(); return Ok((l.port, l.key.clone(), l.ctx)); }
    l.stop();
    let exe = engine_path("llama-server").ok_or_else(|| anyhow!("The AI engine (llama-server) is missing from the app."))?;
    if !model.is_file() { bail!("No AI language model is installed. Open Settings → AI Models."); }
    let size = std::fs::metadata(model).map(|m| m.len()).unwrap_or(0) as f64 / 1_073_741_824.0;
    if size > crate::sysinfo::memory_gb() * 0.75 { bail!("This model ({size:.1} GB) is too large for this computer's memory. Choose a smaller model in Settings → AI Models."); }
    let port = std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
    let key = uuid::Uuid::new_v4().to_string();
    let ctx = context_for(model);
    let mut cmd = Command::new(exe);
    cmd.args(["-m", &model.display().to_string(), "--host", "127.0.0.1", "--port", &port.to_string(), "--api-key", &key,
              "-c", &ctx.to_string(), "--parallel", "1", "-t", &threads().to_string(), "--jinja", "--no-webui", "--offline",
              "--reasoning-budget", "0", "-fa", "auto", "-ngl", if gpu_available() { "999" } else { "0" }])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).kill_on_drop(true);
    no_window(&mut cmd);
    let mut child = cmd.spawn()?;
    let stderr = child.stderr.take().unwrap();
    let tail = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let t2 = tail.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(l)) = lines.next_line().await { let mut t = t2.lock().unwrap(); t.push(l); if t.len() > 40 { t.remove(0); } }
    });
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            let msg = tail.lock().unwrap().join("\n");
            let low = msg.to_lowercase();
            if low.contains("failed to allocate") || low.contains("out of memory") { bail!("Not enough free memory for this model. Close other apps or choose a smaller model."); }
            bail!("The local AI model could not be started: {}", msg.lines().rev().take(4).collect::<Vec<_>>().join(" | "));
        }
        let ok = HTTP.get(format!("http://127.0.0.1:{port}/health")).timeout(Duration::from_secs(2)).send().await.map(|r| r.status().is_success()).unwrap_or(false);
        if ok { break; }
        if Instant::now() > deadline { let _ = child.start_kill(); bail!("Timed out while loading the AI model."); }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    crate::paths::log(&format!("llama-server ready on 127.0.0.1:{port} ({}, ctx {ctx})", model.display()));
    l.child = Some(child); l.port = port; l.key = key.clone(); l.model = Some(model.to_path_buf()); l.ctx = ctx; l.last_used = Instant::now();
    Ok((port, key, ctx))
}

/// Unloads the model after `minutes` of inactivity to give memory back.
pub async fn unload_if_idle(minutes: u64) {
    let mut l = LLAMA.lock().await;
    if l.model.is_some() && l.last_used.elapsed() > Duration::from_secs(minutes * 60) { l.stop(); }
}

fn body(messages: &[(String, String)], max_tokens: usize, temperature: f64, schema: Option<&Value>, stream: bool) -> Value {
    let mut b = json!({
        "messages": messages.iter().map(|(r, c)| json!({"role": r, "content": c})).collect::<Vec<_>>(),
        "max_tokens": max_tokens, "temperature": temperature, "top_p": 0.9, "repeat_penalty": 1.05,
        "stream": stream, "cache_prompt": true, "chat_template_kwargs": {"enable_thinking": false}
    });
    if let Some(s) = schema { b["response_format"] = json!({"type": "json_schema", "json_schema": {"name": "result", "schema": s}}); }
    b
}

pub async fn complete(model: &Path, messages: &[(String, String)], max_tokens: usize, temperature: f64, schema: Option<&Value>) -> Result<String> {
    let (port, key, _) = ensure_llama(model).await?;
    let r = HTTP.post(format!("http://127.0.0.1:{port}/v1/chat/completions")).bearer_auth(&key)
        .json(&body(messages, max_tokens, temperature, schema, false)).send().await?;
    let status = r.status();
    let v: Value = r.json().await?;
    if !status.is_success() { bail!("The local AI model returned an error: {}", v["error"]["message"].as_str().unwrap_or("unknown")); }
    LLAMA.lock().await.last_used = Instant::now();
    Ok(crate::ai::strip_think(v["choices"][0]["message"]["content"].as_str().unwrap_or("")))
}

pub async fn stream(model: &Path, messages: &[(String, String)], max_tokens: usize, cancel: &tokio::sync::watch::Receiver<bool>,
                    mut on_token: impl FnMut(&str)) -> Result<String> {
    use futures_util::StreamExt;
    let (port, key, _) = ensure_llama(model).await?;
    let r = HTTP.post(format!("http://127.0.0.1:{port}/v1/chat/completions")).bearer_auth(&key)
        .json(&body(messages, max_tokens, 0.2, None, true)).send().await?;
    if !r.status().is_success() { bail!("The local AI model returned an error ({})", r.status()); }
    let mut s = r.bytes_stream();
    let (mut buf, mut full) = (String::new(), String::new());
    while let Some(chunk) = s.next().await {
        if *cancel.borrow() { break; }
        buf.push_str(&String::from_utf8_lossy(&chunk?));
        while let Some(i) = buf.find('\n') {
            let line = buf[..i].trim().to_string();
            buf.drain(..=i);
            let Some(payload) = line.strip_prefix("data:") else { continue };
            let payload = payload.trim();
            if payload == "[DONE]" { break; }
            if let Ok(v) = serde_json::from_str::<Value>(payload) {
                if let Some(t) = v["choices"][0]["delta"]["content"].as_str() { full.push_str(t); on_token(t); }
            }
        }
    }
    LLAMA.lock().await.last_used = Instant::now();
    Ok(crate::ai::strip_think(&full))
}

pub async fn token_count(model: &Path, text: &str) -> usize {
    let Ok((port, key, _)) = ensure_llama(model).await else { return crate::ai::est(text) };
    let r = HTTP.post(format!("http://127.0.0.1:{port}/tokenize")).bearer_auth(&key).json(&json!({"content": text})).send().await;
    match r { Ok(r) => r.json::<Value>().await.ok().and_then(|v| v["tokens"].as_array().map(|a| a.len())).unwrap_or_else(|| crate::ai::est(text)),
              Err(_) => crate::ai::est(text) }
}

pub async fn context_size() -> usize { LLAMA.lock().await.ctx }
