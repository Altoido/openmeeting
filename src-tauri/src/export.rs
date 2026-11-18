//! Exports from local data only: Markdown, plain text, SRT, WebVTT, Word (.docx).
//! (PDF is produced by the app's print view → "Save as PDF", which works on macOS and Windows.)
use crate::{ai, db::Db};
use anyhow::Result;
use std::io::Write;

fn date(ts: f64) -> String {
    chrono::DateTime::from_timestamp(ts as i64, 0).map(|d| d.with_timezone(&chrono::Local).format("%B %-d, %Y %-I:%M %p").to_string()).unwrap_or_default()
}
pub fn subtitle(d: &Db, id: &str) -> String {
    d.meeting(id).map(|m| format!("{} · {} min{}", date(m.created_at), (m.duration / 60.0).round(), if m.tags.is_empty() { String::new() } else { format!(" · {}", m.tags.join(", ")) })).unwrap_or_default()
}

pub fn transcript_md(d: &Db, id: &str) -> String {
    let ls = ai::lines(&d.segments(id), &d.speakers(id), 90.0);
    if ls.is_empty() { return "_No transcript yet._".into(); }
    ls.iter().map(|l| format!("**[{}]**{} {}", ai::stamp(l.start), l.speaker.as_ref().map(|s| format!(" **{s}:**")).unwrap_or_default(), l.text)).collect::<Vec<_>>().join("\n\n")
}
pub fn actions_md(d: &Db, id: &str) -> String {
    let items = d.action_items(Some(id));
    if items.is_empty() { return "_No action items._".into(); }
    items.iter().map(|a| format!("- [{}] **{}** — Owner: {} — Due: {}{}", if a.done { "x" } else { " " }, a.task,
        if a.owner.is_empty() { "not specified" } else { &a.owner }, if a.due.is_empty() { "not specified" } else { &a.due },
        a.source_time.map(|t| format!(" [{}]", ai::stamp(t))).unwrap_or_default())).collect::<Vec<_>>().join("\n")
}

/// content: tldr | summary | transcript | actions | everything
pub fn body(d: &Db, id: &str, content: &str) -> String {
    let summary = d.summaries(id).first().map(|s| s.markdown.clone()).unwrap_or_else(|| "_No AI summary yet._".into());
    let notes = d.meeting(id).map(|m| m.user_notes).filter(|n| !n.trim().is_empty()).map(|n| format!("\n\n## My Notes\n\n{n}")).unwrap_or_default();
    let hl = d.highlights(id);
    let hl = if hl.is_empty() { String::new() } else { format!("\n\n## Highlights\n\n{}", hl.iter().map(|h| format!("- [{}] {}", ai::stamp(h.time), h.label)).collect::<Vec<_>>().join("\n")) };
    match content {
        "tldr" => d.tldr(id).map(|t| t.0).unwrap_or_else(|| "_No TL;DR yet._".into()),
        "transcript" => format!("## Transcript\n\n{}", transcript_md(d, id)),
        "actions" => format!("## Action Items\n\n{}", actions_md(d, id)),
        "everything" => format!("{summary}{notes}{hl}\n\n---\n\n## Transcript\n\n{}", transcript_md(d, id)),
        _ => format!("{summary}{notes}{hl}"),
    }
}

pub fn markdown(d: &Db, id: &str, content: &str) -> String {
    let title = d.meeting(id).map(|m| m.title).unwrap_or_default();
    format!("# {title}\n\n_{}_\n\n{}\n", subtitle(d, id), body(d, id, content))
}

fn plain_inline(s: &str) -> String { s.replace("**", "").replace('`', "") }
pub fn plain_text(md: &str) -> String {
    md.lines().map(|l| {
        let t = l.trim_end();
        if let Some(h) = t.strip_prefix("# ").or_else(|| t.strip_prefix("## ")) { let h = plain_inline(h).to_uppercase(); format!("\n{h}\n{}", "=".repeat(h.chars().count().min(60))) }
        else if let Some(h) = t.strip_prefix("### ") { plain_inline(h).to_uppercase() }
        else { plain_inline(&t.replacen("- [ ]", "[ ]", 1).replacen("- [x]", "[x]", 1).replacen("- ", "• ", 1)) }
    }).collect::<Vec<_>>().join("\n").trim().to_string() + "\n"
}

fn ts(t: f64, vtt: bool) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}{}{:03}", ms / 3_600_000, ms % 3_600_000 / 60_000, ms % 60_000 / 1000, if vtt { '.' } else { ',' }, ms % 1000)
}
pub fn subtitles(d: &Db, id: &str, vtt: bool) -> String {
    let names = d.speakers(id);
    let mut out = if vtt { "WEBVTT\n\n".to_string() } else { String::new() };
    for (i, s) in d.segments(id).iter().enumerate() {
        let who = s.speaker.as_ref().map(|k| format!("{}: ", names.get(k).cloned().unwrap_or_else(|| ai::speaker_default_name(k)))).unwrap_or_default();
        if !vtt { out += &format!("{}\n", i + 1); }
        out += &format!("{} --> {}\n{who}{}\n\n", ts(s.start, vtt), ts(s.end.max(s.start + 0.5), vtt), s.text);
    }
    out
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").chars().filter(|c| *c == '\t' || *c == '\n' || *c >= ' ').collect()
}
fn runs(text: &str, force_bold: bool) -> String {
    text.split("**").enumerate().filter(|(_, p)| !p.is_empty()).map(|(i, p)| {
        let b = force_bold || i % 2 == 1;
        format!("<w:r>{}<w:t xml:space=\"preserve\">{}</w:t></w:r>", if b { "<w:rPr><w:b/></w:rPr>" } else { "" }, xml(&p.replace('`', "")))
    }).collect()
}
fn para(inner: &str, style: Option<&str>, indent: u32) -> String {
    let mut ppr = String::new();
    if let Some(s) = style { ppr += &format!("<w:pStyle w:val=\"{s}\"/>"); }
    if indent > 0 { ppr += &format!("<w:ind w:left=\"{indent}\" w:hanging=\"260\"/>"); }
    format!("<w:p>{}{inner}</w:p>", if ppr.is_empty() { String::new() } else { format!("<w:pPr>{ppr}</w:pPr>") })
}

pub fn docx(title: &str, subtitle: &str, md: &str) -> Result<Vec<u8>> {
    let mut body = para(&runs(title, false), Some("Title"), 0) + &para(&runs(subtitle, false), Some("Subtitle"), 0);
    let mut table: Vec<Vec<String>> = vec![];
    let flush_table = |table: &mut Vec<Vec<String>>, body: &mut String| {
        if table.is_empty() { return; }
        let mut t = String::from("<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/><w:tblW w:w=\"5000\" w:type=\"pct\"/></w:tblPr>");
        for (ri, row) in table.iter().enumerate() { t += "<w:tr>"; for c in row { t += &format!("<w:tc>{}</w:tc>", para(&runs(c, ri == 0), None, 0)); } t += "</w:tr>"; }
        *body += &(t + "</w:tbl>"); table.clear();
    };
    for line in md.lines() {
        let t = line.trim();
        let indent = (line.len() - line.trim_start().len()) as u32 / 2;
        if t.starts_with('|') {
            let cells: Vec<String> = t.trim_matches('|').split('|').map(|c| c.trim().to_string()).collect();
            if !cells.iter().all(|c| !c.is_empty() && c.chars().all(|x| x == '-' || x == ':')) { table.push(cells); }
            continue;
        }
        flush_table(&mut table, &mut body);
        if t.is_empty() { continue; }
        if let Some(h) = t.strip_prefix("### ") { body += &para(&runs(h, false), Some("Heading2"), 0); }
        else if let Some(h) = t.strip_prefix("## ").or_else(|| t.strip_prefix("# ")) { body += &para(&runs(h, false), Some("Heading1"), 0); }
        else if let Some(b) = t.strip_prefix("- [ ] ") { body += &para(&runs(&format!("☐\t{b}"), false), None, 360 + indent * 360); }
        else if let Some(b) = t.strip_prefix("- [x] ") { body += &para(&runs(&format!("☑\t{b}"), false), None, 360 + indent * 360); }
        else if let Some(b) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) { body += &para(&runs(&format!("•\t{b}"), false), None, 360 + indent * 360); }
        else if let Some(q) = t.strip_prefix('>') { body += &para(&runs(q.trim(), false), Some("Quote"), 0); }
        else if t == "---" { body += &para("", None, 0); }
        else { body += &para(&runs(t, false), None, 0); }
    }
    flush_table(&mut table, &mut body);
    let doc = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1080" w:right="1080" w:bottom="1080" w:left="1080" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#);
    let styles = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri"/><w:sz w:val="22"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="120"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:rPr><w:b/><w:sz w:val="40"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Subtitle"><w:name w:val="Subtitle"/><w:pPr><w:spacing w:after="240"/></w:pPr><w:rPr><w:color w:val="666666"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:pPr><w:keepNext/><w:spacing w:before="320" w:after="120"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:color w:val="1F3A5F"/><w:sz w:val="30"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:pPr><w:keepNext/><w:spacing w:before="200" w:after="60"/><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:b/><w:sz w:val="24"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/><w:pPr><w:ind w:left="720"/></w:pPr><w:rPr><w:i/></w:rPr></w:style><w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:color="BFBFBF"/><w:left w:val="single" w:sz="4" w:color="BFBFBF"/><w:bottom w:val="single" w:sz="4" w:color="BFBFBF"/><w:right w:val="single" w:sz="4" w:color="BFBFBF"/><w:insideH w:val="single" w:sz="4" w:color="BFBFBF"/><w:insideV w:val="single" w:sz="4" w:color="BFBFBF"/></w:tblBorders></w:tblPr></w:style></w:styles>"#;
    let ct = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let drels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let o = zip::write::SimpleFileOptions::default();
        for (n, c) in [("[Content_Types].xml", ct.to_string()), ("_rels/.rels", rels.into()), ("word/document.xml", doc), ("word/styles.xml", styles.into()), ("word/_rels/document.xml.rels", drels.into())] {
            z.start_file(n, o)?; z.write_all(c.as_bytes())?;
        }
        z.finish()?;
    }
    Ok(buf.into_inner())
}

pub fn export(d: &Db, id: &str, content: &str, format: &str) -> Result<Vec<u8>> {
    let title = d.meeting(id).map(|m| m.title).unwrap_or_default();
    Ok(match format {
        "md" => markdown(d, id, content).into_bytes(),
        "txt" => plain_text(&markdown(d, id, content)).into_bytes(),
        "srt" => subtitles(d, id, false).into_bytes(),
        "vtt" => subtitles(d, id, true).into_bytes(),
        "docx" => docx(&title, &subtitle(d, id), &body(d, id, content))?,
        f => anyhow::bail!("Unknown format {f}"),
    })
}
