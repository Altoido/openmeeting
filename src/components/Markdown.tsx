import React from "react";

type Block = { t: "h"; level: number; text: string } | { t: "li"; indent: number; text: string; checked: boolean | null }
  | { t: "ol"; n: string; text: string } | { t: "quote"; text: string } | { t: "table"; rows: string[][] } | { t: "hr" } | { t: "p"; text: string };

export function parse(md: string): Block[] {
  const out: Block[] = []; let para: string[] = []; let table: string[][] = [];
  const fp = () => { if (para.length) { out.push({ t: "p", text: para.join(" ") }); para = []; } };
  const ft = () => { if (table.length) { out.push({ t: "table", rows: table }); table = []; } };
  for (const raw of md.replace(/\r\n/g, "\n").split("\n")) {
    const indent = raw.length - raw.trimStart().length; const l = raw.trim();
    if (!l) { fp(); ft(); continue; }
    if (l.startsWith("|")) { fp(); const cells = l.replace(/^\||\|$/g, "").split("|").map(c => c.trim()); if (!cells.every(c => /^[-:]+$/.test(c))) table.push(cells); continue; } else ft();
    let m = l.match(/^(#{1,6})\s+(.*)$/); if (m) { fp(); out.push({ t: "h", level: m[1].length, text: m[2] }); continue; }
    if (/^(-{3,}|\*{3,}|_{3,})$/.test(l)) { fp(); out.push({ t: "hr" }); continue; }
    if (l.startsWith(">")) { fp(); out.push({ t: "quote", text: l.slice(1).trim() }); continue; }
    m = l.match(/^[-*+•]\s+(.*)$/); if (m) { fp(); let body = m[1]; let checked: boolean | null = null;
      if (/^\[ \]/.test(body)) { checked = false; body = body.slice(3).trim(); } else if (/^\[x\]/i.test(body)) { checked = true; body = body.slice(3).trim(); }
      out.push({ t: "li", indent: Math.floor(indent / 2), text: body, checked }); continue; }
    m = l.match(/^(\d+)[.)]\s+(.*)$/); if (m) { fp(); out.push({ t: "ol", n: m[1], text: m[2] }); continue; }
    para.push(l);
  }
  fp(); ft(); return out;
}

export function wordCount(md: string) { return md.replace(/[*_#>`[\]|]/g, " ").split(/\s+/).filter(w => /[\p{L}\p{N}]/u.test(w)).length; }

function inline(text: string, onSeek?: (t: number) => void): React.ReactNode[] {
  const parts: React.ReactNode[] = []; const re = /(\*\*[^*]+\*\*|`[^`]+`|\*[^*\s][^*]*\*|\[\d{1,2}:\d{2}(?::\d{2})?\])/g;
  let last = 0; let m: RegExpExecArray | null; let k = 0;
  while ((m = re.exec(text))) {
    if (m.index > last) parts.push(text.slice(last, m.index));
    const s = m[0];
    if (s.startsWith("**")) parts.push(<strong key={k++}>{s.slice(2, -2)}</strong>);
    else if (s.startsWith("`")) parts.push(<code key={k++}>{s.slice(1, -1)}</code>);
    else if (s.startsWith("[")) { const p = s.slice(1, -1).split(":").map(Number); const t = p.length === 3 ? p[0] * 3600 + p[1] * 60 + p[2] : p[0] * 60 + p[1];
      parts.push(onSeek ? <a key={k++} className="stamp" onClick={() => onSeek(t)}>{s}</a> : <span key={k++} className="stamp">{s}</span>); }
    else parts.push(<em key={k++}>{s.slice(1, -1)}</em>);
    last = m.index + s.length;
  }
  if (last < text.length) parts.push(text.slice(last));
  return parts;
}

export function Markdown({ md, onSeek }: { md: string; onSeek?: (t: number) => void }) {
  return <div className="md">{parse(md).map((b, i) => {
    switch (b.t) {
      case "h": return b.level <= 2 ? <h2 key={i}>{inline(b.text, onSeek)}</h2> : <h3 key={i}>{inline(b.text, onSeek)}</h3>;
      case "li": return <div key={i} className="li" style={{ marginLeft: b.indent * 18 }}><span className="bullet">{b.checked === null ? "•" : b.checked ? "☑" : "☐"}</span><span>{inline(b.text, onSeek)}</span></div>;
      case "ol": return <div key={i} className="li"><span className="bullet">{b.n}.</span><span>{inline(b.text, onSeek)}</span></div>;
      case "quote": return <blockquote key={i}>{inline(b.text, onSeek)}</blockquote>;
      case "table": return <table key={i}><tbody>{b.rows.map((r, ri) => <tr key={ri}>{r.map((c, ci) => ri === 0 ? <th key={ci}>{inline(c)}</th> : <td key={ci}>{inline(c, onSeek)}</td>)}</tr>)}</tbody></table>;
      case "hr": return <hr key={i} />;
      default: return <p key={i}>{inline(b.text, onSeek)}</p>;
    }
  })}</div>;
}
