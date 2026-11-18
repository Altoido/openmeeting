//! Model catalog, downloads (only when the user clicks Download), verification and import.
use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use sha1::Digest as _;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModelSpec {
    pub id: String, pub kind: String, pub name: String, pub file_name: String, pub url: String,
    pub approx_bytes: u64, pub sha1: Option<String>, pub license: String, pub summary: String,
    pub context: usize, pub min_ram_gb: f64,
}

fn hf(repo: &str, file: &str) -> String { format!("https://huggingface.co/{repo}/resolve/main/{file}") }

fn spec(id: &str, kind: &str, name: &str, repo: &str, file: &str, bytes: u64, sha1: Option<&str>, license: &str, summary: &str, ctx: usize, ram: f64) -> ModelSpec {
    ModelSpec { id: id.into(), kind: kind.into(), name: name.into(), file_name: file.into(), url: hf(repo, file), approx_bytes: bytes,
                sha1: sha1.map(String::from), license: license.into(), summary: summary.into(), context: ctx, min_ram_gb: ram }
}

pub fn catalog() -> Vec<ModelSpec> {
    let w = "ggerganov/whisper.cpp";
    vec![
        spec("whisper-base", "whisper", "Fast (Whisper Base)", w, "ggml-base.bin", 148_000_000, Some("465707469ff3a37a2b9b8d8f89f2f99de7299dac"), "MIT", "Quick drafts, any language. Runs well on every computer.", 0, 0.0),
        spec("whisper-small", "whisper", "Balanced (Whisper Small)", w, "ggml-small.bin", 488_000_000, Some("55356645c2b361a969dfd0ef2c5a50d530afd8d5"), "MIT", "Good accuracy, any language. Recommended for Windows PCs and Intel Macs.", 0, 0.0),
        spec("whisper-large-v3-turbo-q5_0", "whisper", "High Accuracy (Whisper Large v3 Turbo, compressed)", w, "ggml-large-v3-turbo-q5_0.bin", 574_000_000, Some("e050f7970618a659205450ad97eb95a18d69c9ee"), "MIT", "Near-best accuracy. Fast on Apple Silicon; slower on Windows.", 0, 0.0),
        spec("vad-silero", "vad", "Silence detection (Silero VAD)", "ggml-org/whisper-vad", "ggml-silero-v6.2.0.bin", 900_000, None, "MIT", "Skips long silences so transcription is faster and cleaner.", 0, 0.0),
        spec("qwen3-1.7b", "llm", "Compact (Qwen3 1.7B)", "unsloth/Qwen3-1.7B-GGUF", "Qwen3-1.7B-Q4_K_M.gguf", 1_110_000_000, None, "Apache 2.0", "For 8 GB computers. Basic summaries.", 8192, 8.0),
        spec("qwen3-4b-2507", "llm", "Recommended (Qwen3 4B Instruct 2507)", "unsloth/Qwen3-4B-Instruct-2507-GGUF", "Qwen3-4B-Instruct-2507-Q4_K_M.gguf", 2_500_000_000, None, "Apache 2.0", "Detailed, well-organized notes. Best for 16–32 GB.", 16384, 12.0),
        spec("qwen3-30b-a3b-2507", "llm", "Premium (Qwen3 30B-A3B Instruct 2507)", "unsloth/Qwen3-30B-A3B-Instruct-2507-GGUF", "Qwen3-30B-A3B-Instruct-2507-Q4_K_M.gguf", 18_600_000_000, None, "Apache 2.0", "Highest quality. Needs 36 GB+ memory (Apple Silicon recommended).", 32768, 36.0),
    ]
}

pub fn recommended() -> (String, String) {
    let gb = crate::sysinfo::memory_gb();
    let fast_gpu = crate::engines::gpu_available();
    let whisper = if fast_gpu && gb >= 12.0 { "whisper-large-v3-turbo-q5_0" } else { "whisper-small" };
    let llm = if gb >= 36.0 && fast_gpu { "qwen3-30b-a3b-2507" } else if gb >= 12.0 { "qwen3-4b-2507" } else { "qwen3-1.7b" };
    (whisper.into(), llm.into())
}

pub fn dir_for(kind: &str) -> PathBuf { crate::paths::model_dir(kind) }

pub fn installed(kind: &str) -> Vec<PathBuf> {
    let ext = if kind == "llm" { "gguf" } else { "bin" };
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir_for(kind)).map(|it| it.flatten().map(|e| e.path())
        .filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case(ext)).unwrap_or(false)).collect()).unwrap_or_default();
    v.sort();
    v
}

/// Returns None if the file looks valid, otherwise a user-facing problem.
pub fn verify(path: &Path, kind: &str, sha1: Option<&str>, sha256: Option<&str>) -> Option<String> {
    let mut f = match std::fs::File::open(path) { Ok(f) => f, Err(_) => return Some("The file cannot be opened.".into()) };
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() { return Some("The file is empty or damaged.".into()); }
    match kind {
        "llm" if &magic != b"GGUF" => return Some("This is not a GGUF language-model file.".into()),
        "whisper" | "vad" if magic != [0x6c, 0x6d, 0x67, 0x67] => return Some("This is not a whisper.cpp (ggml .bin) model file.".into()),
        _ => {}
    }
    if sha1.is_none() && sha256.is_none() { return None; }
    let mut f = std::fs::File::open(path).ok()?;
    let (mut h1, mut h256) = (sha1::Sha1::new(), sha2::Sha256::new());
    let mut buf = vec![0u8; 8 << 20];
    loop {
        let n = f.read(&mut buf).unwrap_or(0);
        if n == 0 { break; }
        if sha1.is_some() { h1.update(&buf[..n]); }
        if sha256.is_some() { sha2::Digest::update(&mut h256, &buf[..n]); }
    }
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    if let Some(s) = sha1 { if hex(&h1.finalize()) != s.to_lowercase() { return Some("The download is damaged (checksum mismatch). Please try again.".into()); } }
    if let Some(s) = sha256 { if hex(&sha2::Digest::finalize(h256)) != s.to_lowercase() { return Some("The download is damaged (checksum mismatch). Please try again.".into()); } }
    None
}

/// Hugging Face reports the SHA-256 of large files in the X-Linked-Etag header of the redirect.
async fn expected_sha256(url: &str) -> Option<String> {
    let c = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().ok()?;
    let r = c.head(url).send().await.ok()?;
    let tag = r.headers().get("x-linked-etag")?.to_str().ok()?.trim_matches(|c| c == '"' || c == 'W' || c == '/').to_lowercase();
    (tag.len() == 64 && tag.chars().all(|c| c.is_ascii_hexdigit())).then_some(tag)
}

pub async fn download(spec: &ModelSpec, cancel: tokio::sync::watch::Receiver<bool>, mut progress: impl FnMut(u64, u64)) -> Result<PathBuf> {
    use futures_util::StreamExt;
    let free = crate::sysinfo::free_disk_gb() * 1_073_741_824.0;
    if (free as u64) < spec.approx_bytes + 500_000_000 { bail!("Not enough free disk space for {} (needs about {:.1} GB).", spec.name, spec.approx_bytes as f64 / 1e9); }
    let sha256 = expected_sha256(&spec.url).await;
    let resp = reqwest::get(&spec.url).await?;
    if !resp.status().is_success() { bail!("The server answered with error {}. Try “Import model file” instead.", resp.status()); }
    let total = resp.content_length().unwrap_or(spec.approx_bytes);
    let tmp = crate::paths::models().join(format!(".download-{}", uuid::Uuid::new_v4()));
    let mut file = std::fs::File::create(&tmp)?;
    let mut got = 0u64;
    let mut s = resp.bytes_stream();
    while let Some(chunk) = s.next().await {
        if *cancel.borrow() { drop(file); let _ = std::fs::remove_file(&tmp); bail!("cancelled"); }
        let chunk = chunk.map_err(|e| anyhow!("Download failed: {e}. Check your connection or import a model file instead."))?;
        file.write_all(&chunk)?;
        got += chunk.len() as u64;
        progress(got, total);
    }
    drop(file);
    let (tmp2, kind, s1) = (tmp.clone(), spec.kind.clone(), spec.sha1.clone());
    let problem = tokio::task::spawn_blocking(move || verify(&tmp2, &kind, s1.as_deref(), sha256.as_deref())).await?;
    if let Some(p) = problem { let _ = std::fs::remove_file(&tmp); bail!(p); }
    let dest = dir_for(&spec.kind).join(&spec.file_name);
    let _ = std::fs::remove_file(&dest);
    std::fs::rename(&tmp, &dest)?;
    Ok(dest)
}

pub fn import(src: &Path, kind: &str) -> Result<PathBuf> {
    if let Some(p) = verify(src, kind, None, None) { bail!(p); }
    let dest = dir_for(kind).join(src.file_name().ok_or_else(|| anyhow!("bad file name"))?);
    std::fs::copy(src, &dest)?;
    Ok(dest)
}
