//! Offline AI pipeline (same design and prompt files as the original macOS app, which were tested
//! end-to-end with the real llama-server): extract → hierarchical merge (with retention check) →
//! template compose → TL;DR → title; plus Ask AI with FTS5 retrieval and mind maps.
use crate::db::Db;
use crate::engines;
use crate::models::*;
use anyhow::{anyhow, Result};
use once_cell::sync::OnceCell;
use regex::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub static PROMPTS_DIR: OnceCell<PathBuf> = OnceCell::new();

pub fn prompt(name: &str) -> String {
    let dir = PROMPTS_DIR.get().cloned().unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/prompts"));
    std::fs::read_to_string(dir.join(name)).unwrap_or_default()
}
pub fn fill(name: &str, vals: &[(&str, &str)]) -> String {
    let mut t = prompt(name);
    for (k, v) in vals { t = t.replace(&format!("{{{{{k}}}}}"), v); }
    t
}
pub fn system() -> String { prompt("system.txt").trim().to_string() }
pub fn schema(key: &str) -> Option<Value> { serde_json::from_str::<Value>(&prompt("schemas.json")).ok()?.get(key).cloned() }

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Template { pub id: String, pub name: String, #[serde(default)] pub icon: String, #[serde(default)] pub description: String, pub instructions: String }

pub fn templates(db: &Db) -> Vec<Template> {
    let mut t: Vec<Template> = serde_json::from_str(&prompt("templates.json")).unwrap_or_default();
    for (id, name, ins) in db.custom_templates() {
        t.push(Template { id, name, icon: "custom".into(), description: "Custom template".into(),
            instructions: format!("Organize the document according to these instructions from the user. Choose clear \"## \" section headings that fit them:\n{ins}") });
    }
    t
}
pub fn template(db: &Db, id: &str) -> Template {
    let all = templates(db);
    all.iter().find(|t| t.id == id).cloned().or_else(|| all.first().cloned())
        .unwrap_or(Template { id: "general".into(), name: "General Meeting Notes".into(), icon: String::new(), description: String::new(), instructions: "## Summary\n## Decisions\n## Action Items".into() })
}

// ---------------------------------------------------------------- text helpers
pub fn est(text: &str) -> usize { ((text.len() as f64) / 3.3).ceil() as usize }
pub fn stamp(s: f64) -> String { let s = s.max(0.0) as u64; format!("{:02}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60) }
pub fn strip_think(t: &str) -> String {
    let re = Regex::new(r"(?s)<think>.*?(</think>|$)").unwrap();
    re.replace_all(t, "").trim().to_string()
}
pub fn extract_json(t: &str) -> String {
    let t = t.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    match (t.find(['{', '[']), t.rfind(['}', ']'])) { (Some(a), Some(b)) if b > a => t[a..=b].to_string(), _ => t.to_string() }
}
pub fn speaker_default_name(key: &str) -> String {
    match key { "ME" => "Me".into(), "REMOTE" => "Remote participants".into(),
        k if k.starts_with('S') && k[1..].parse::<u32>().is_ok() => format!("Speaker {}", &k[1..]), k => k.into() }
}

#[derive(Clone, Debug)]
pub struct Line { pub start: f64, pub end: f64, pub speaker: Option<String>, pub text: String }
impl Line { pub fn render(&self) -> String { format!("[{}] {}{}", stamp(self.start), self.speaker.as_ref().map(|s| format!("{s}: ")).unwrap_or_default(), self.text) } }

pub fn lines(segs: &[Segment], names: &HashMap<String, String>, max_line: f64) -> Vec<Line> {
    let mut out: Vec<Line> = vec![];
    for s in segs {
        let who = s.speaker.as_ref().map(|k| names.get(k).filter(|n| !n.trim().is_empty()).cloned().unwrap_or_else(|| speaker_default_name(k)));
        let text = s.text.trim();
        if text.is_empty() { continue; }
        if let Some(last) = out.last_mut() {
            if last.speaker == who && s.start - last.end < 2.0 && s.end - last.start <= max_line { last.text.push(' '); last.text.push_str(text); last.end = s.end; continue; }
        }
        out.push(Line { start: s.start, end: s.end, speaker: who, text: text.into() });
    }
    out
}
pub fn render(lines: &[Line]) -> String { lines.iter().map(|l| l.render()).collect::<Vec<_>>().join("\n") }

pub fn split(lines: &[Line], max_tokens: usize, ratio: f64) -> Vec<Vec<Line>> {
    let budget = max_tokens.max(256);
    let (mut chunks, mut cur, mut used) = (vec![], vec![], 0usize);
    for l in lines {
        let c = ((est(&l.render()) + 1) as f64 * ratio) as usize;
        if used + c > budget && !cur.is_empty() { chunks.push(std::mem::take(&mut cur)); used = 0; }
        cur.push(l.clone()); used += c;
    }
    if !cur.is_empty() { chunks.push(cur); }
    chunks
}

fn words(s: &str) -> HashSet<String> { s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 2).map(String::from).collect() }
pub fn similar(a: &str, b: &str) -> bool {
    let (wa, wb) = (words(a), words(b));
    if wa.is_empty() || wb.is_empty() { return a.eq_ignore_ascii_case(b); }
    wa.intersection(&wb).count() as f64 / wa.len().min(wb.len()) as f64 >= 0.6
}
/// Re-adds any decision/task/question the model dropped while merging parts.
pub fn ensure_retained(mut m: Notes, inputs: &[Notes]) -> Notes {
    for n in inputs {
        for d in &n.decisions { if !m.decisions.iter().any(|x| similar(&x.decision, &d.decision)) { m.decisions.push(d.clone()); } }
        for a in &n.action_items { if !m.action_items.iter().any(|x| similar(&x.task, &a.task)) { m.action_items.push(a.clone()); } }
        for q in &n.open_questions { if !m.open_questions.iter().any(|x| similar(x, q)) { m.open_questions.push(q.clone()); } }
        for q in &n.next_steps { if !m.next_steps.iter().any(|x| similar(x, q)) { m.next_steps.push(q.clone()); } }
    }
    if m.topics.is_empty() { m.topics = inputs.iter().flat_map(|n| n.topics.clone()).collect(); }
    if m.executive_summary.trim().is_empty() { m.executive_summary = inputs.iter().map(|n| n.executive_summary.clone()).collect::<Vec<_>>().join(" "); }
    if m.purpose.trim().is_empty() { m.purpose = inputs.iter().find(|n| !n.purpose.is_empty()).map(|n| n.purpose.clone()).unwrap_or_default(); }
    if m.quotes.is_empty() { m.quotes = inputs.iter().flat_map(|n| n.quotes.clone()).take(8).collect(); }
    m
}
pub fn compact(n: &Notes, max_tokens: usize) -> Notes {
    let mut c = n.clone();
    let mut per = 12usize;
    while est(&serde_json::to_string(&c).unwrap_or_default()) > max_tokens && per > 1 {
        per -= 2;
        c.topics = n.topics.iter().map(|t| Topic { title: t.title.clone(), details: t.details.iter().take(per.max(1)).cloned().collect() }).collect();
        c.quotes = n.quotes.iter().take(4).cloned().collect();
    }
    c
}
/// Removes values small models put in the wrong field (a [timestamp] as a due date, "null" as an owner).
pub fn sanitize(mut n: Notes) -> Notes {
    let stamp_re = Regex::new(r"^\[?\d{1,2}:\d{2}(:\d{2})?\]?$").unwrap();
    let junk = ["null", "none", "n/a", "unknown", "not specified", "unspecified", "not mentioned", "tbd"];
    let clean = |v: &Option<String>| v.as_ref().map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && !junk.contains(&s.to_lowercase().as_str()));
    for a in n.action_items.iter_mut() {
        a.owner = clean(&a.owner).filter(|s| !stamp_re.is_match(s));
        a.due = clean(&a.due).filter(|s| !stamp_re.is_match(s));
        a.priority = clean(&a.priority);
    }
    n.action_items.retain(|a| !a.task.trim().is_empty());
    n.decisions.retain(|d| !d.decision.trim().is_empty());
    n
}

/// Repairs small-model formatting slips: fences, repeated title, "### ## Heading", copied guidance, echoed rules.
pub fn clean_markdown(raw: &str, tpl: &Template) -> String {
    let mut t = raw.trim().to_string();
    if t.starts_with("```") {
        let mut ls: Vec<&str> = t.lines().skip(1).collect();
        if ls.last().map(|l| l.trim().starts_with("```")).unwrap_or(false) { ls.pop(); }
        t = ls.join("\n");
    }
    let ins: Vec<String> = tpl.instructions.lines().map(|l| l.trim().to_string()).collect();
    let heads: Vec<String> = ins.iter().filter(|l| l.starts_with("## ")).map(|l| l[3..].trim().to_string()).collect();
    let guidance: Vec<String> = ins.iter().filter(|l| l.starts_with('(') && l.ends_with(')')).cloned().collect();
    let echoed = ["general rules", "rules", "meeting information", "extracted notes", "format to follow"];
    let (mut out, mut first) = (vec![], true);
    for line in t.lines() {
        let tl = line.trim();
        if guidance.iter().any(|g| g == tl) { continue; }
        if tl.starts_with('(') && tl.ends_with(')') && guidance.iter().any(|g| similar(g, tl)) { continue; }
        let mut line = line.to_string();
        if tl.starts_with('#') {
            let mut level = tl.len() - tl.trim_start_matches('#').len();
            let mut text = tl.trim_start_matches('#').trim().to_string();
            if text.starts_with('#') { text = text.trim_start_matches('#').trim().to_string(); level = 2; }
            let low = text.to_lowercase();
            if echoed.iter().any(|e| low.starts_with(e)) { break; }
            if first && level <= 2 && (low == tpl.name.to_lowercase() || level == 1) && !heads.iter().any(|h| h.to_lowercase() == low) { first = false; continue; }
            if let Some(h) = heads.iter().find(|h| low.starts_with(&h.to_lowercase()) && (level <= 2 || low == h.to_lowercase())) { text = h.clone(); level = 2; }
            else if !heads.is_empty() && level <= 2 { level = 3; }
            line = format!("{} {}", "#".repeat(level.max(2)), text);
        }
        if !tl.is_empty() { first = false; }
        out.push(line);
    }
    out.join("\n").trim().to_string()
}

pub const TLDR_MAX_WORDS: usize = 300;
pub fn limit_words(md: &str, max: usize) -> String {
    let (mut out, mut used, mut cut) = (Vec::<&str>::new(), 0usize, false);
    for line in md.lines() {
        let n = line.split_whitespace().count();
        if used + n > max { cut = true; break; }
        out.push(line); used += n;
    }
    while out.last().map(|l| l.trim().is_empty() || l.trim().starts_with('#')).unwrap_or(false) { out.pop(); }
    let mut r = out.join("\n");
    if cut { r.push_str("\n\n_Shortened to fit on one page — see the full Summary for everything._"); }
    r
}

pub fn hash(parts: &[&str]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    for p in parts { h.update(p.as_bytes()); h.update([0x1f]); }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------- pipeline
type SharedDb = Arc<Mutex<Db>>;

fn msgs(user: String) -> Vec<(String, String)> { vec![("system".into(), system()), ("user".into(), user)] }

async fn notes_call(model: &Path, prompt: String, max_tokens: usize) -> Result<Notes> {
    let schema = schema("notes");
    let mut last = anyhow!("unknown");
    for attempt in 0..2 {
        match engines::complete(model, &msgs(prompt.clone()), max_tokens, if attempt == 0 { 0.2 } else { 0.05 }, schema.as_ref()).await {
            Ok(out) => match serde_json::from_str::<Notes>(&extract_json(&out)) { Ok(n) => return Ok(sanitize(n)), Err(e) => last = anyhow!("The model's notes could not be read: {e}") },
            Err(e) => last = e,
        }
    }
    Err(last)
}

pub struct SummaryResult { pub summary: Summary, pub notes: Notes, pub tldr: Option<String>, pub title: Option<String>, pub parts: usize }

pub async fn summarize(db: SharedDb, mid: &str, tpl: &Template, want_title: bool, model: &Path, progress: impl Fn(&str, f64)) -> Result<SummaryResult> {
    let (meeting, segs, names) = { let d = db.lock().unwrap(); (d.meeting(mid).ok_or_else(|| anyhow!("meeting not found"))?, d.segments(mid), d.speakers(mid)) };
    if segs.is_empty() { return Err(anyhow!("This meeting has no transcript yet.")); }
    let ls = lines(&segs, &names, 45.0);
    let transcript = render(&ls);
    progress("Loading local AI model…", 0.02);
    let (_, _, ctx) = engines::ensure_llama(model).await?;
    let real = engines::token_count(model, &transcript).await;
    let ratio = (real as f64 / est(&transcript).max(1) as f64).clamp(0.5, 2.0);
    let map_reserve = (ctx / 4).clamp(1200, 3000);
    let chunk_budget = ctx.saturating_sub(1100 + map_reserve + 200);
    let h = hash(&[&transcript, &model.display().to_string(), &prompt("extract.txt"), &prompt("merge.txt")]);

    let chunks = split(&ls, chunk_budget, ratio);
    let mut parts = vec![];
    for (i, ch) in chunks.iter().enumerate() {
        progress(&if chunks.len() > 1 { format!("Reading part {} of {}…", i + 1, chunks.len()) } else { "Reading the transcript…".into() }, 0.05 + 0.55 * i as f64 / chunks.len() as f64);
        if let Some(n) = db.lock().unwrap().chunk_note(mid, 0, i as i64, &h).and_then(|j| serde_json::from_str::<Notes>(&j).ok()) { parts.push(n); continue; }
        let range = format!("{} to {}", stamp(ch[0].start), stamp(ch[ch.len() - 1].end));
        let ctxline = if chunks.len() > 1 { "(The meeting was split into parts because it is long; extract only from this part.)\n" } else { "" };
        let n = notes_call(model, fill("extract.txt", &[("PART", &(i + 1).to_string()), ("TOTAL", &chunks.len().to_string()), ("RANGE", &range),
                                                         ("CONTEXT", ctxline), ("TRANSCRIPT", &render(ch))]), map_reserve).await?;
        db.lock().unwrap().save_chunk_note(mid, 0, i as i64, &h, &serde_json::to_string(&n)?);
        parts.push(n);
    }
    let mut level = 1i64;
    while parts.len() > 1 {
        progress(&format!("Combining notes from {} parts…", parts.len()), 0.62);
        let mut groups: Vec<Vec<Notes>> = vec![];
        let (mut g, mut used) = (vec![], 0usize);
        for n in &parts {
            let t = (est(&serde_json::to_string(n)?) as f64 * ratio) as usize;
            if used + t > chunk_budget && !g.is_empty() { groups.push(std::mem::take(&mut g)); used = 0; }
            g.push(n.clone()); used += t;
        }
        if !g.is_empty() { groups.push(g); }
        if groups.len() == parts.len() { groups = parts.chunks(2).map(|c| c.iter().map(|n| compact(n, chunk_budget / 2)).collect()).collect(); }
        let mut next = vec![];
        for (gi, g) in groups.iter().enumerate() {
            if g.len() == 1 { next.push(g[0].clone()); continue; }
            if let Some(n) = db.lock().unwrap().chunk_note(mid, level, gi as i64, &h).and_then(|j| serde_json::from_str::<Notes>(&j).ok()) { next.push(n); continue; }
            let payload = g.iter().enumerate().map(|(i, n)| format!("PART {}:\n{}", i + 1, serde_json::to_string(n).unwrap_or_default())).collect::<Vec<_>>().join("\n\n");
            let merged = match notes_call(model, fill("merge.txt", &[("COUNT", &g.len().to_string()), ("NOTES", &payload)]), map_reserve).await {
                Ok(m) => m, Err(e) => { crate::paths::log(&format!("merge failed, deterministic merge: {e}")); Notes::default() } };
            let merged = ensure_retained(merged, g);
            db.lock().unwrap().save_chunk_note(mid, level, gi as i64, &h, &serde_json::to_string(&merged)?);
            next.push(merged);
        }
        parts = next; level += 1;
        if level > 8 { parts = vec![ensure_retained(Notes::default(), &parts)]; }
    }
    let notes = parts.pop().unwrap_or_default();

    progress(&format!("Writing {}…", tpl.name), 0.75);
    let compose_reserve = (ctx / 3).clamp(1500, 4096);
    let budget = ctx.saturating_sub(1100 + compose_reserve + est(&tpl.instructions) + 200);
    let mut nfc = notes.clone();
    let mut ntok = (est(&serde_json::to_string(&notes)?) as f64 * ratio) as usize;
    if ntok > budget { nfc = compact(&notes, (budget as f64 / ratio) as usize); ntok = (est(&serde_json::to_string(&nfc)?) as f64 * ratio) as usize; }
    let user_notes = meeting.user_notes.trim();
    let include = chunks.len() == 1 && real + ntok + est(user_notes) <= budget;
    let speakers: Vec<String> = { let mut s: Vec<String> = ls.iter().filter_map(|l| l.speaker.clone()).collect::<HashSet<_>>().into_iter().collect(); s.sort(); s };
    let date = chrono::DateTime::from_timestamp(meeting.created_at as i64, 0).map(|d| d.with_timezone(&chrono::Local).format("%B %-d, %Y %-I:%M %p").to_string()).unwrap_or_default();
    let meta = format!("Title: {}\nDate: {}\nDuration: {} min\nSpeakers in transcript: {}", meeting.title, date, (meeting.duration / 60.0).round(),
                       if speakers.is_empty() { "not labeled".into() } else { speakers.join(", ") });
    let un = if user_notes.is_empty() { String::new() } else { format!("\nNOTES TYPED BY THE USER DURING THE MEETING (treat as accurate):\n{user_notes}\n") };
    let tr = if include { format!("\nFULL TRANSCRIPT:\n{transcript}\n") } else { String::new() };
    let p = fill("compose.txt", &[("META", &meta), ("TEMPLATE_NAME", &tpl.name), ("TEMPLATE", &tpl.instructions),
        ("TRANSCRIPT_CLAUSE", if include { " and the transcript" } else { "" }), ("NOTES", &serde_json::to_string(&nfc)?), ("USER_NOTES", &un), ("TRANSCRIPT", &tr)]);
    let md = clean_markdown(&engines::complete(model, &msgs(p), compose_reserve, 0.3, None).await?, tpl);

    progress("Writing TL;DR…", 0.9);
    let tldr = tldr(model, &notes, &meta, user_notes, ctx).await.ok();
    let mut title = None;
    if want_title { progress("Suggesting a title…", 0.95); title = suggest_title(model, &notes).await.ok().flatten(); }
    let summary = Summary { id: 0, template_id: tpl.id.clone(), template_name: tpl.name.clone(), markdown: md,
        structured_json: serde_json::to_string(&notes)?, model_name: model.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(), created_at: crate::db::now() };
    Ok(SummaryResult { summary, notes, tldr, title, parts: chunks.len() })
}

pub fn tldr_template() -> Template {
    Template { id: "tldr".into(), name: "TL;DR".into(), icon: String::new(), description: String::new(), instructions: "## Key Points\n## Decisions\n## Action Items\n## Open Questions".into() }
}

pub async fn tldr(model: &Path, notes: &Notes, meta: &str, user_notes: &str, ctx: usize) -> Result<String> {
    let c = compact(notes, (ctx / 2).max(1500));
    let un = if user_notes.is_empty() { String::new() } else { format!("\nNOTES TYPED BY THE USER DURING THE MEETING:\n{}\n", user_notes.chars().take(2000).collect::<String>()) };
    let p = fill("tldr.txt", &[("MAX_WORDS", &(TLDR_MAX_WORDS - 40).to_string()), ("META", meta), ("NOTES", &serde_json::to_string(&c)?), ("USER_NOTES", &un)]);
    let raw = engines::complete(model, &msgs(p), 700, 0.2, None).await?;
    Ok(limit_words(&clean_markdown(&raw, &tldr_template()), TLDR_MAX_WORDS))
}

pub async fn suggest_title(model: &Path, notes: &Notes) -> Result<Option<String>> {
    let brief = format!("Summary: {}\nPurpose: {}\nTopics: {}", notes.executive_summary, notes.purpose, notes.topics.iter().map(|t| t.title.clone()).collect::<Vec<_>>().join("; "));
    let out = engines::complete(model, &msgs(fill("title.txt", &[("NOTES", &brief)])), 60, 0.2, schema("title").as_ref()).await?;
    let v: Value = serde_json::from_str(&extract_json(&out))?;
    Ok(v["title"].as_str().map(|t| t.trim().trim_matches('"').to_string()).filter(|t| !t.is_empty() && t.len() < 90))
}

pub async fn mindmap(model: &Path, material: &str) -> Result<MindNode> {
    let out = engines::complete(model, &msgs(fill("mindmap.txt", &[("NOTES", material)])), 1500, 0.2, schema("mindmap").as_ref()).await?;
    Ok(serde_json::from_str(&extract_json(&out))?)
}

pub fn action_items(notes: &Notes, mid: &str) -> Vec<ActionItem> {
    notes.action_items.iter().filter(|a| !a.task.trim().is_empty()).map(|a| ActionItem {
        id: uuid::Uuid::new_v4().to_string(), meeting_id: mid.into(), task: a.task.trim().into(), owner: a.owner.clone().unwrap_or_default(),
        due: a.due.clone().unwrap_or_default(), priority: a.priority.clone().unwrap_or_default(), done: false, notes: String::new(),
        source_time: a.timestamp.as_deref().and_then(parse_stamp) }).collect()
}
pub fn parse_stamp(s: &str) -> Option<f64> {
    let n: Vec<f64> = s.trim_matches(|c| c == '[' || c == ']' || c == ' ').split(':').map(|p| p.parse::<f64>()).collect::<Result<_, _>>().ok()?;
    match n.len() { 3 => Some(n[0] * 3600.0 + n[1] * 60.0 + n[2]), 2 => Some(n[0] * 60.0 + n[1]), _ => None }
}

// ---------------------------------------------------------------- Ask AI
const STOP: &[&str] = &["the","and","for","with","about","what","which","who","whom","when","where","why","how","did","does","are","was","were","been",
    "our","you","your","they","them","their","its","this","that","these","those","there","any","all","can","could","would","should","will","please",
    "tell","give","list","summarize","summary","meeting","meetings","said","say","says","discussed","discuss","mentioned","mention","talk","talked",
    "everything","anything","something","from","has","have","had","write","make","get","also","just","some","into","than","not"];
pub fn keywords(q: &str) -> String {
    q.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() >= 3 && !STOP.contains(w)).collect::<Vec<_>>().join(" ")
}

pub fn select_windows(ls: &[Line], hits: &[f64], budget: usize) -> Vec<Line> {
    let (mut chosen, mut seen, mut used) = (std::collections::BTreeSet::new(), HashSet::new(), 0usize);
    'outer: for &t in hits {
        let idx = ls.iter().rposition(|l| l.start <= t + 0.01).unwrap_or(0);
        if !seen.insert(idx) { continue; }
        for j in idx.saturating_sub(2)..=(idx + 2).min(ls.len().saturating_sub(1)) {
            if chosen.contains(&j) { continue; }
            let c = est(&ls[j].render()) + 1;
            if used + c > budget { break 'outer; }
            chosen.insert(j); used += c;
        }
    }
    let mut out = vec![];
    let mut prev: Option<usize> = None;
    for i in chosen {
        if let Some(p) = prev { if i > p + 1 { out.push(Line { start: ls[i].start, end: ls[i].start, speaker: None, text: "…".into() }); } }
        out.push(ls[i].clone()); prev = Some(i);
    }
    out
}

fn notes_block(d: &Db, m: &Meeting, max_tokens: usize) -> String {
    let mut parts = vec![];
    if let Some(s) = d.summaries(&m.id).first() { parts.push(format!("AI SUMMARY ({}):\n{}", s.template_name, s.markdown.chars().take(max_tokens * 3).collect::<String>())); }
    let acts = d.action_items(Some(&m.id));
    if !acts.is_empty() {
        parts.push(format!("ACTION ITEMS:\n{}", acts.iter().map(|a| format!("- {}{}{}{}", a.task, if a.owner.is_empty() { String::new() } else { format!(" (owner: {})", a.owner) },
            if a.due.is_empty() { String::new() } else { format!(" (due: {})", a.due) }, if a.done { " [done]" } else { "" })).collect::<Vec<_>>().join("\n")));
    }
    if !m.user_notes.trim().is_empty() { parts.push(format!("NOTES TYPED BY THE USER:\n{}", m.user_notes)); }
    parts.join("\n\n")
}

pub fn material(d: &Db, q: &str, mid: Option<&str>, budget: usize) -> (String, String) {
    let header = |m: &Meeting| format!("=== Meeting: {} — {} ===", m.title, chrono::DateTime::from_timestamp(m.created_at as i64, 0).map(|x| x.with_timezone(&chrono::Local).format("%b %-d, %Y").to_string()).unwrap_or_default());
    if let Some(mid) = mid {
        let Some(m) = d.meeting(mid) else { return (String::new(), String::new()) };
        let ls = lines(&d.segments(mid), &d.speakers(mid), 45.0);
        let notes = notes_block(d, &m, budget / 4);
        let full = render(&ls);
        if est(&full) + est(&notes) <= budget {
            return (format!("{}\n{}\n\nFULL TRANSCRIPT:\n{}", header(&m), notes, full), "You have the complete transcript of this meeting.".into());
        }
        let mut hits: Vec<f64> = d.search(&keywords(q), Some(&["segment"]), Some(mid), 60, true).iter().filter_map(|h| h.r#ref.parse().ok()).collect();
        let lq = q.to_lowercase();
        hits.extend(ls.iter().filter(|l| l.speaker.as_ref().map(|s| lq.contains(&s.to_lowercase())).unwrap_or(false)).map(|l| l.start));
        let sel = select_windows(&ls, &hits, budget.saturating_sub(est(&notes)));
        return (format!("{}\n{}\n\nRELEVANT TRANSCRIPT EXCERPTS (chronological; '…' separates gaps):\n{}", header(&m), notes,
                        if sel.is_empty() { "(No transcript passages matched the question's keywords.)".into() } else { render(&sel) }),
                "This meeting is long, so you have its summary plus the transcript passages most relevant to the question. If the answer is not in them, say it was not found in the available excerpts.".into());
    }
    let hits = d.search(&keywords(q), Some(&["segment", "summary", "action", "title", "notes"]), None, 150, true);
    let mut order: Vec<String> = vec![];
    for h in &hits { if !order.contains(&h.meeting_id) { order.push(h.meeting_id.clone()); } }
    if order.is_empty() { order = d.meetings().into_iter().filter(|m| !m.is_archived).take(6).map(|m| m.id).collect(); }
    let per = (budget / order.len().clamp(1, 8)).max(600);
    let (mut blocks, mut used) = (vec![], 0);
    for id in order.iter().take(8) {
        let Some(m) = d.meeting(id) else { continue };
        let mut b = format!("{}\n{}", header(&m), notes_block(d, &m, per / 2));
        let times: Vec<f64> = hits.iter().filter(|h| &h.meeting_id == id && h.kind == "segment").filter_map(|h| h.r#ref.parse().ok()).collect();
        if !times.is_empty() { b += &format!("\nTRANSCRIPT EXCERPTS:\n{}", render(&select_windows(&lines(&d.segments(id), &d.speakers(id), 45.0), &times, per / 2))); }
        if used + est(&b) > budget { break; }
        used += est(&b); blocks.push(b);
    }
    (if blocks.is_empty() { "(No saved meetings matched.)".into() } else { blocks.join("\n\n") },
     "You have excerpts from the saved meetings most relevant to the question (searched across the whole library). Mention which meeting (title and date) each point comes from.".into())
}

pub async fn ask(db: SharedDb, q: &str, mid: Option<&str>, model: &Path, cancel: &tokio::sync::watch::Receiver<bool>, on_token: impl FnMut(&str)) -> Result<String> {
    let (_, _, ctx) = engines::ensure_llama(model).await?;
    let history: Vec<ChatMessage> = { let d = db.lock().unwrap(); let h = d.chat(mid); h[h.len().saturating_sub(7)..h.len().saturating_sub(1)].to_vec() };
    let reserve = (ctx / 4).min(2000);
    let budget = ctx.saturating_sub(900 + reserve + history.iter().map(|h| est(&h.content)).sum::<usize>() + est(q)).max(1000);
    let (mat, coverage) = { let d = db.lock().unwrap(); material(&d, q, mid, budget) };
    let sys = format!("{}\n\n{}", system(), fill("ask.txt", &[("MULTI_CITE", if mid.is_none() { " and the meeting title" } else { "" }),
        ("SCOPE", if mid.is_none() { "saved meetings" } else { "meeting" }), ("COVERAGE", &format!("\n{coverage}\n")), ("MATERIAL", &mat)]));
    let mut m: Vec<(String, String)> = vec![("system".into(), sys)];
    for h in history { m.push((h.role, h.content)); }
    m.push(("user".into(), q.into()));
    engines::stream(model, &m, reserve, cancel, on_token).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean_and_limit() {
        let tpl = Template { id: "t".into(), name: "General Meeting Notes".into(), icon: String::new(), description: String::new(), instructions: "## Overview\n(summary.)\n## Action Items\n(a checklist.)".into() };
        let raw = "## General Meeting Notes\n\n### ## Overview\n(summary.)\nText\n## Action Items — a checklist.\n- [ ] x\n## Extra\ny\n## General rules:\n- junk";
        assert_eq!(clean_markdown(raw, &tpl), "## Overview\nText\n## Action Items\n- [ ] x\n### Extra\ny");
        let long = format!("**Bottom line:** done.\n## Key Points\n{}\n## Decisions\n- x", (0..100).map(|i| format!("- point {i} with five words")).collect::<Vec<_>>().join("\n"));
        let cut = limit_words(&long, 300);
        assert!(cut.ends_with("see the full Summary for everything._") && !cut.contains("## Decisions"));
    }
    #[test]
    fn retention_and_sanitize() {
        let a = Notes { action_items: vec![Action { task: "Set up the registration form".into(), owner: Some("Jen".into()), ..Default::default() }], ..Default::default() };
        let b = Notes { action_items: vec![Action { task: "Call the food bank about a truck".into(), owner: Some("null".into()), due: Some("[00:01:31]".into()), ..Default::default() }], ..Default::default() };
        let merged = Notes { action_items: vec![Action { task: "Set up registration form".into(), ..Default::default() }], ..Default::default() };
        let r = sanitize(ensure_retained(merged, &[a, b]));
        assert_eq!(r.action_items.len(), 2);
        assert!(r.action_items[1].owner.is_none() && r.action_items[1].due.is_none());
    }
    #[test]
    fn whisper_json_and_windows() {
        let j = br#"{"transcription":[{"offsets":{"from":0,"to":2000},"text":" Hello world."},{"offsets":{"from":2000,"to":3000},"text":" [BLANK_AUDIO]"}]}"#;
        let s = crate::engines::parse_whisper_json(j).unwrap();
        assert_eq!(s.len(), 1); assert_eq!(s[0].text, "Hello world.");
        let ls: Vec<Line> = (0..50).map(|i| Line { start: i as f64 * 10.0, end: i as f64 * 10.0 + 9.0, speaker: None, text: format!("line {i}") }).collect();
        let sel = select_windows(&ls, &[100.0, 400.0], 500);
        assert!(sel.iter().any(|l| l.text == "line 10") && sel.iter().any(|l| l.text == "line 40") && sel.iter().any(|l| l.text == "…"));
    }

    /// Live end-to-end test with the real engine. Run with:
    /// OMN_TEST_LLM=/path/model.gguf OMN_ENGINES_DIR=/path/bin cargo test --lib live_pipeline -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_pipeline() {
        let Ok(model) = std::env::var("OMN_TEST_LLM") else { return };
        let dir = std::env::temp_dir().join(format!("omn-live-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("OMN_DATA_DIR", &dir);
        let db = Arc::new(Mutex::new(Db::open(&dir.join("t.sqlite")).unwrap()));
        let fx: Value = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/staff_meeting.json")).unwrap()).unwrap();
        {
            let d = db.lock().unwrap();
            d.save_meeting(&Meeting { id: "m".into(), title: "Recording".into(), duration: 220.0, status: "transcribed".into(), source: "recording".into(),
                                      language: "auto".into(), template_id: "general".into(), ..Default::default() }).unwrap();
            let segs: Vec<(f64, f64, String, Option<String>)> = fx["segments"].as_array().unwrap().iter().map(|s| (s["start"].as_f64().unwrap(), s["end"].as_f64().unwrap(),
                s["text"].as_str().unwrap().to_string(), s["speaker"].as_str().map(String::from))).collect();
            d.append_segments("m", &segs).unwrap();
            for (k, v) in fx["speakers"].as_object().unwrap() { d.set_speaker("m", k, v.as_str().unwrap()); }
            d.reindex_segments("m");
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let tpl = template(&db.lock().unwrap(), "general");
            let r = summarize(db.clone(), "m", &tpl, true, Path::new(&model), |s, f| eprintln!("  [{:>3.0}%] {s}", f * 100.0)).await.unwrap();
            eprintln!("--- SUMMARY ---\n{}\n--- TL;DR ---\n{}\n--- TITLE: {:?}", r.summary.markdown, r.tldr.clone().unwrap_or_default(), r.title);
            let heads: Vec<&str> = r.summary.markdown.lines().filter(|l| l.starts_with("## ")).collect();
            assert!(heads.len() >= 4, "headings {heads:?}");
            let tl = r.tldr.unwrap();
            assert!(tl.split_whitespace().count() <= TLDR_MAX_WORDS + 15 && tl.contains("## Action Items"));
            assert!(!r.notes.action_items.is_empty());
            db.lock().unwrap().add_summary("m", &r.summary);
            db.lock().unwrap().add_chat(Some("m"), "user", "What did we decide about the parking lot repaving?");
            let (_tx, rx) = tokio::sync::watch::channel(false);
            let mut n = 0;
            let a = ask(db.clone(), "What did we decide about the parking lot repaving?", Some("m"), Path::new(&model), &rx, |_| n += 1).await.unwrap();
            eprintln!("--- ASK ---\n{a}");
            assert!(n > 3);
            let mm = mindmap(Path::new(&model), &r.summary.markdown).await.unwrap();
            assert!(!mm.children.is_empty());
            let docx = crate::export::docx("T", "S", &r.summary.markdown).unwrap();
            assert_eq!(&docx[..2], b"PK");
            engines::LLAMA.lock().await.stop();
        });
    }
}
