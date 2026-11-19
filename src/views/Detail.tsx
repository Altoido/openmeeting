import { useEffect, useMemo, useRef, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { save, confirm } from "@tauri-apps/plugin-dialog";
import { api, ActionItem, Detail, Template, clock, duration, dateText, speakerName, errText, approx } from "../api";
import { Markdown, wordCount } from "../components/Markdown";
import { MindMap } from "../components/MindMap";
import { AskView } from "./Ask";
import { Ctx, statusLabel } from "../App";

const TABS = ["TL;DR", "Summary", "Transcript", "Action Items", "Ask AI", "Mind Map", "Notes"] as const;
type Tab = typeof TABS[number];

/** Renders Markdown into the print area and opens the system print dialog (→ Save as PDF). */
export async function printDoc(title: string, subtitle: string, md: string) {
  const { createRoot } = await import("react-dom/client");
  const el = document.getElementById("print-root")!;
  const root = createRoot(el);
  root.render(<div className="printdoc"><h1>{title}</h1><div className="sub">{subtitle}</div><Markdown md={md} /></div>);
  await new Promise(r => setTimeout(r, 150));
  try { await invoke("print_page"); } catch { window.print(); }
  setTimeout(() => root.unmount(), 1000);
}

export function MeetingDetail({ id, ctx, onDeleted }: { id: string; ctx: Ctx; onDeleted: () => void }) {
  const [d, setD] = useState<Detail | null>(null);
  const [tab, setTab] = useState<Tab>("TL;DR");
  const [title, setTitle] = useState("");
  const [showExport, setShowExport] = useState(false);
  const audio = useRef<HTMLAudioElement>(null);
  const [time, setTime] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [rate, setRate] = useState(1);
  const [peaks, setPeaks] = useState<number[]>([]);
  const [est, setEst] = useState<number | null>(null);

  const load = () => api.detail(id).then(x => { setD(x); setTitle(x.meeting.title); });
  useEffect(() => { load(); api.peaks(id).then(setPeaks).catch(() => {}); }, [id]);
  useEffect(() => { load(); }, [ctx.meetings]);
  useEffect(() => { if (d && ["recorded", "interrupted", "queued", "failed"].includes(d.meeting.status)) api.estimate(id).then(e => setEst(e.seconds)).catch(() => {}); }, [d?.meeting.status, d?.meeting.duration]);
  useEffect(() => { if (audio.current) audio.current.playbackRate = rate; }, [rate]);
  if (!d) return <div className="center muted">Loading…</div>;
  const m = d.meeting;
  const busy = ctx.progress.meetingId === id;
  const queued = ctx.progress.queued.includes(id);
  const hasLlm = !!ctx.sys && true;
  const seek = (t: number) => { if (audio.current) { audio.current.currentTime = t; audio.current.play(); } };
  const save_ = async (patch: Partial<typeof m>) => { await api.save({ ...m, ...patch }); await ctx.reload(); };
  const dur = audio.current?.duration && isFinite(audio.current.duration) ? audio.current.duration : m.duration;

  return <div className="detailwrap">
    <div className="dhead">
      <div className="titlerow">
        <input className="title" value={title} onChange={e => setTitle(e.target.value)} onBlur={() => title.trim() && title !== m.title && save_({ title: title.trim(), titleUserSet: true })} />
        <button className="icon" title="Favorite" onClick={() => save_({ isFavorite: !m.isFavorite })}>{m.isFavorite ? "★" : "☆"}</button>
        <button onClick={() => setShowExport(true)}>Export…</button>
        <MoreMenu d={d} ctx={ctx} onDeleted={onDeleted} save_={save_} />
      </div>
      <div className="muted small">{dateText(m.createdAt)} · {duration(m.duration)} · {m.source === "imported" ? m.originalFileName : "Recorded"}{m.stereoSources ? " · Me + remote" : ""}</div>
      <TagEditor tags={m.tags} onChange={tags => save_({ tags })} />
      {(busy || queued || m.status === "recorded") && ((busy ? ctx.progress.etaSeconds : est) ?? 0) > 1800 &&
        <div className="tip">⏾ This will take about {approx((busy ? ctx.progress.etaSeconds : est) ?? 0)} — fine to leave running overnight. Keep the computer plugged in; it stays awake while processing (closing a laptop lid may still put it to sleep).</div>}
      {busy ? <div className="notice"><div className="bar wide"><div style={{ width: `${ctx.progress.fraction * 100}%` }} /></div><span>{ctx.progress.stage}</span>{ctx.progress.etaSeconds ? <span className="muted small">about {approx(ctx.progress.etaSeconds)} left</span> : null}<span className="local" title="The computer is kept awake until processing finishes.">🔒 on this computer · staying awake</span><button onClick={() => api.cancel(id)}>Cancel</button></div>
        : queued ? <div className="notice">Queued — another meeting is being processed. <button onClick={() => api.cancel(id)}>Cancel</button></div>
        : m.status === "recorded" ? <div className="notice">This recording hasn't been transcribed yet.{est ? ` Estimated time on this computer: about ${approx(est)}.` : ""} <button className="primary" onClick={() => api.process(id, "full")}>Transcribe & Summarize</button></div>
        : m.status === "interrupted" ? <div className="notice warn">{m.statusDetail || "Processing was interrupted."} <button className="primary" onClick={() => api.process(id, "full")}>Resume</button></div>
        : m.status === "failed" ? <div className="notice err">{m.statusDetail} <button className="primary" onClick={() => api.process(id, "full")}>Retry</button>{d.segments.length > 0 && <button onClick={() => api.process(id, "summarize", m.templateId)}>Retry Summary Only</button>}</div>
        : m.status === "transcribed" ? <div className="notice">{m.statusDetail || "Transcript ready — no AI summary yet."} <button className="primary" onClick={() => api.process(id, "summarize", m.templateId)}>Summarize</button></div>
        : null}
    </div>
    {d.audioPath && <div className="player">
      <audio ref={audio} src={convertFileSrc(d.audioPath)} onTimeUpdate={e => setTime(e.currentTarget.currentTime)} onPlay={() => setPlaying(true)} onPause={() => setPlaying(false)} preload="metadata" />
      <button className="icon" onClick={() => audio.current && (audio.current.currentTime -= 10)}>↺10</button>
      <button className="play" onClick={() => audio.current && (playing ? audio.current.pause() : audio.current.play())}>{playing ? "❚❚" : "▶"}</button>
      <button className="icon" onClick={() => audio.current && (audio.current.currentTime += 10)}>10↻</button>
      <span className="mono">{clock(time)}</span>
      <Waveform peaks={peaks} progress={dur ? time / dur : 0} marks={dur ? d.highlights.map(h => h.time / dur) : []} onSeek={f => seek(f * dur)} />
      <span className="mono muted">{clock(dur)}</span>
      <select value={rate} onChange={e => setRate(Number(e.target.value))}>{[0.75, 1, 1.25, 1.5, 1.75, 2].map(r => <option key={r} value={r}>{r}×</option>)}</select>
    </div>}
    <div className="tabs">{TABS.map(t => <button key={t} className={tab === t ? "active" : ""} onClick={() => setTab(t)}>{t}</button>)}</div>
    <div className="tabbody">
      {tab === "TL;DR" && <TldrTab d={d} ctx={ctx} reload={load} />}
      {tab === "Summary" && <SummaryTab d={d} ctx={ctx} reload={load} seek={seek} />}
      {tab === "Transcript" && <TranscriptTab d={d} reload={load} seek={seek} time={time} busy={busy} />}
      {tab === "Action Items" && <ActionsTab d={d} reload={load} seek={seek} />}
      {tab === "Ask AI" && <AskView ctx={ctx} meetingId={id} onSeek={seek} />}
      {tab === "Mind Map" && <MindMapTab d={d} ctx={ctx} reload={load} />}
      {tab === "Notes" && <NotesTab d={d} reload={load} seek={seek} />}
    </div>
    {showExport && <ExportModal d={d} onClose={() => setShowExport(false)} ctx={ctx} />}
  </div>;
}

function MoreMenu({ d, ctx, onDeleted, save_ }: { d: Detail; ctx: Ctx; onDeleted: () => void; save_: (p: any) => Promise<void> }) {
  const [open, setOpen] = useState(false);
  const m = d.meeting;
  return <div className="menuwrap"><button onClick={() => setOpen(!open)}>⋯</button>{open && <div className="menu" onMouseLeave={() => setOpen(false)}>
    <button onClick={() => { api.process(m.id, "full"); setOpen(false); }}>Transcribe & Summarize</button>
    <button onClick={async () => { if (await confirm("Re-transcribe this meeting from the beginning and rebuild its summary and TL;DR? Edits to the transcript will be replaced.")) api.process(m.id, "transcribe"); setOpen(false); }}>Re-transcribe from Beginning</button>
    <div className="menulabel">Move to collection</div>
    <button onClick={() => save_({ collectionId: null })}>None</button>
    {ctx.collections.map(c => <button key={c.id} onClick={() => save_({ collectionId: c.id })}>{c.name}{m.collectionId === c.id ? " ✓" : ""}</button>)}
    <hr />
    <button onClick={() => save_({ isArchived: !m.isArchived })}>{m.isArchived ? "Unarchive" : "Archive"}</button>
    <button onClick={() => d.audioPath && invoke("plugin:opener|reveal_item_in_dir", { path: d.audioPath })}>Show Audio File</button>
    <button className="danger" onClick={async () => { if (await confirm(`Delete “${m.title}” permanently? The audio and everything generated from it will be removed.`, { kind: "warning" })) { await api.remove(m.id); onDeleted(); ctx.reload(); } }}>Delete…</button>
  </div>}</div>;
}

function TagEditor({ tags, onChange }: { tags: string[]; onChange: (t: string[]) => void }) {
  const [v, setV] = useState("");
  return <div className="tags">{tags.map(t => <span key={t} className="tag">#{t} <a onClick={() => onChange(tags.filter(x => x !== t))}>×</a></span>)}
    <input placeholder="Add tag" value={v} onChange={e => setV(e.target.value)} onKeyDown={e => { if (e.key === "Enter" && v.trim()) { onChange([...new Set([...tags, v.trim().replace(/^#/, "")])]); setV(""); } }} /></div>;
}

function Waveform({ peaks, progress, marks, onSeek }: { peaks: number[]; progress: number; marks: number[]; onSeek: (f: number) => void }) {
  return <div className="wave" onClick={e => { const r = e.currentTarget.getBoundingClientRect(); onSeek((e.clientX - r.left) / r.width); }}>
    {peaks.length ? peaks.map((p, i) => <div key={i} className={i / peaks.length <= progress ? "on" : ""} style={{ height: `${Math.max(4, p * 100)}%` }} />)
      : <div className="track"><div style={{ width: `${progress * 100}%` }} /></div>}
    {marks.map((m, i) => <span key={`m${i}`} className="mark" style={{ left: `${m * 100}%` }} />)}
  </div>;
}

function TldrTab({ d, ctx, reload }: { d: Detail; ctx: Ctx; reload: () => void }) {
  const [busy, setBusy] = useState(false); const [err, setErr] = useState(""); const [edit, setEdit] = useState<string | null>(null);
  const md = d.tldr?.markdown;
  const gen = async () => { setBusy(true); setErr(""); try { await api.tldr(d.meeting.id); reload(); } catch (e) { setErr(errText(e)); } setBusy(false); };
  if (!md) return <div className="empty">
    <h3>No TL;DR yet</h3>
    <p className="muted">{d.summaries.length ? "Create a one-page TL;DR from this meeting's notes. It takes a few seconds and runs on this computer." : d.segments.length ? "The TL;DR is written automatically together with the full summary." : "The TL;DR is written automatically with the summary once the meeting is transcribed."}</p>
    {d.summaries.length > 0 ? <button className="primary" disabled={busy} onClick={gen}>{busy ? "Writing…" : "Create TL;DR"}</button>
      : d.segments.length > 0 && <button className="primary" onClick={() => api.process(d.meeting.id, "summarize", d.meeting.templateId)}>Generate Summary & TL;DR</button>}
    {err && <p className="errtext">{err}</p>}
  </div>;
  const words = wordCount(edit ?? md);
  return <div>
    <div className="toolbar">
      <b>TL;DR</b><span className={words <= 300 ? "muted small" : "warntext small"}>{words} words · {words <= 300 ? "fits on one page" : "longer than one page"}</span>
      {busy && <span className="muted small">Writing on this computer…</span>}{err && <span className="errtext small">{err}</span>}
      <div className="grow" />
      {edit === null ? <>
        <button disabled={busy || !d.summaries.length} onClick={gen}>Regenerate</button>
        <button onClick={() => setEdit(md)}>Edit</button>
        <button onClick={() => navigator.clipboard.writeText(md)}>Copy</button>
        <button className="primary" onClick={() => printDoc(d.meeting.title, `${dateText(d.meeting.createdAt)} · ${duration(d.meeting.duration)}`, md)}>One-Page PDF</button>
      </> : <><button onClick={() => setEdit(null)}>Cancel</button><button className="primary" onClick={async () => { await api.saveTldr(d.meeting.id, edit, d.tldr?.model ?? ""); setEdit(null); reload(); }}>Save</button></>}
    </div>
    <div className="page">
      <h2 className="pagetitle">{d.meeting.title}</h2><div className="muted small">{dateText(d.meeting.createdAt)} · {duration(d.meeting.duration)}</div><hr />
      {edit === null ? <Markdown md={md} /> : <textarea className="editor" value={edit} onChange={e => setEdit(e.target.value)} />}
    </div>
    <p className="muted small center">Generated on this computer by {d.tldr?.model}. See the Summary tab for full details.</p>
  </div>;
}

function SummaryTab({ d, ctx, reload, seek }: { d: Detail; ctx: Ctx; reload: () => void; seek: (t: number) => void }) {
  const [templates, setTemplates] = useState<Template[]>([]);
  const [tpl, setTpl] = useState(d.meeting.templateId);
  const [shown, setShown] = useState(0);
  const [edit, setEdit] = useState<string | null>(null);
  useEffect(() => { api.templates().then(setTemplates); }, []);
  const s = d.summaries[shown] ?? d.summaries[0];
  const queued = ctx.progress.meetingId === d.meeting.id || ctx.progress.queued.includes(d.meeting.id);
  return <div>
    <div className="toolbar">
      <select value={tpl} onChange={e => setTpl(e.target.value)}>{templates.map(t => <option key={t.id} value={t.id}>{t.name}</option>)}</select>
      <button className="primary" disabled={!d.segments.length || queued} onClick={() => api.process(d.meeting.id, "summarize", tpl)}>{d.summaries.length ? "Regenerate" : "Generate Summary"}</button>
      <div className="grow" />
      {d.summaries.length > 1 && <select value={shown} onChange={e => setShown(Number(e.target.value))}>{d.summaries.map((x, i) => <option key={x.id} value={i}>{x.templateName} — {dateText(x.createdAt)}</option>)}</select>}
      {s && (edit === null ? <><button onClick={() => setEdit(s.markdown)}>Edit</button><button onClick={() => navigator.clipboard.writeText(s.markdown)}>Copy</button>
        <button onClick={() => printDoc(d.meeting.title, `${dateText(d.meeting.createdAt)} · ${duration(d.meeting.duration)}`, s.markdown)}>PDF</button></>
        : <><button onClick={() => setEdit(null)}>Cancel</button><button className="primary" onClick={async () => { await api.updateSummary(d.meeting.id, s.id, edit); setEdit(null); reload(); }}>Save</button></>)}
    </div>
    {s ? <div className="doc">{edit === null ? <Markdown md={s.markdown} onSeek={seek} /> : <textarea className="editor tall" value={edit} onChange={e => setEdit(e.target.value)} />}
      <p className="muted small">Generated on this computer by {s.modelName} · {s.templateName}. AI notes can contain mistakes — check important details against the transcript.</p></div>
      : <div className="empty"><h3>No summary yet</h3><p className="muted">{d.segments.length ? "Choose a template and click Generate Summary." : "Transcribe the meeting first; the summary is created automatically afterwards."}</p></div>}
  </div>;
}

function TranscriptTab({ d, reload, seek, time, busy }: { d: Detail; reload: () => void; seek: (t: number) => void; time: number; busy: boolean }) {
  const [q, setQ] = useState(""); const [editing, setEditing] = useState<number | null>(null); const [draft, setDraft] = useState("");
  const [follow, setFollow] = useState(true); const [names, setNames] = useState(d.speakers); const [showSp, setShowSp] = useState(false);
  const keys = useMemo(() => { const k = new Set([...d.segments.map(s => s.speaker).filter(Boolean) as string[], ...Object.keys(d.speakers)]); return k.size ? [...k].sort() : ["S1", "S2"]; }, [d]);
  const current = useMemo(() => { let c = -1; d.segments.forEach(s => { if (s.start <= time + 0.05) c = s.id; }); return c; }, [d.segments, time]);
  useEffect(() => { if (follow && editing === null && !q && current >= 0) document.getElementById(`seg-${current}`)?.scrollIntoView({ block: "center", behavior: "smooth" }); }, [current]);
  const list = q ? d.segments.filter(s => s.text.toLowerCase().includes(q.toLowerCase())) : d.segments;
  const setSpeaker = async (segId: number, key: string | null) => { await api.segment(d.meeting.id, segId, { speaker: key, setSpeaker: true }); reload(); };
  if (!d.segments.length) return <div className="empty"><h3>{busy ? "Transcribing…" : "No transcript yet"}</h3><p className="muted">{busy ? "Text appears when the first part is finished. Long meetings are processed in 10-minute parts." : "Transcription runs on this computer with the local Whisper model."}</p></div>;
  return <div>
    <div className="toolbar">
      <input className="search" placeholder="Find in transcript" value={q} onChange={e => setQ(e.target.value)} />{q && <span className="muted small">{list.length} found</span>}
      <label className="small"><input type="checkbox" checked={follow} onChange={e => setFollow(e.target.checked)} /> Follow playback</label>
      <div className="grow" />
      <button onClick={() => setShowSp(!showSp)}>Speakers</button>
      <button onClick={() => navigator.clipboard.writeText(d.segments.map(s => `[${clock(s.start)}] ${speakerName(s.speaker, d.speakers) ? speakerName(s.speaker, d.speakers) + ": " : ""}${s.text}`).join("\n"))}>Copy</button>
    </div>
    {showSp && <div className="panel">
      <p className="muted small">{d.meeting.stereoSources ? "This recording separated your microphone (“Me”) from computer audio (“Remote participants”). Rename them here, or assign lines to other speakers." : "Automatic voice separation isn't available for single-channel audio. Label lines with the name menu on each line and rename speakers here."}</p>
      {keys.map(k => <div key={k} className="spk"><input placeholder={speakerName(k, {}) ?? k} value={names[k] ?? ""} onChange={e => setNames({ ...names, [k]: e.target.value })} />
        <span className="muted small">{d.segments.filter(s => s.speaker === k).length} lines</span></div>)}
      <button className="primary" onClick={async () => { for (const k of keys) await api.speaker(d.meeting.id, k, names[k]?.trim() || (speakerName(k, {}) ?? k)); setShowSp(false); reload(); }}>Save Names</button>
    </div>}
    <div className="segs">{list.map(s => <div key={s.id} id={`seg-${s.id}`} className={`seg ${s.id === current ? "cur" : ""}`}>
      <a className="stamp" onClick={() => seek(s.start)}>{clock(s.start)}</a>
      <select className="who" value={s.speaker ?? ""} onChange={e => e.target.value === "__new" ? setSpeaker(s.id, `S${Math.max(0, ...keys.filter(k => /^S\d+$/.test(k)).map(k => Number(k.slice(1)))) + 1}`) : setSpeaker(s.id, e.target.value || null)}>
        <option value="">Speaker?</option>{keys.map(k => <option key={k} value={k}>{speakerName(k, d.speakers)}</option>)}<option value="__new">New speaker…</option>
      </select>
      {editing === s.id ? <div className="grow"><textarea className="editor" value={draft} onChange={e => setDraft(e.target.value)} />
          <button onClick={() => setEditing(null)}>Cancel</button><button className="primary" onClick={async () => { if (draft.trim()) await api.segment(d.meeting.id, s.id, { text: draft.trim() }); setEditing(null); reload(); }}>Save</button></div>
        : <div className="segtext" onDoubleClick={() => { setEditing(s.id); setDraft(s.text); }} title="Double-click to edit">{s.text}
          {s.text !== s.originalText && <a className="small muted" title={`Original: ${s.originalText}`} onClick={async () => { await api.segment(d.meeting.id, s.id, { revert: true }); reload(); }}> (edited — revert)</a>}</div>}
    </div>)}</div>
  </div>;
}

function ActionsTab({ d, reload, seek }: { d: Detail; reload: () => void; seek: (t: number) => void }) {
  const [task, setTask] = useState("");
  const add = async () => { if (!task.trim()) return; await api.saveAction({ id: crypto.randomUUID(), meetingId: d.meeting.id, task: task.trim(), owner: "", due: "", priority: "", done: false, notes: "" }); setTask(""); reload(); };
  return <div>
    <div className="toolbar"><input className="search" placeholder="Add an action item…" value={task} onChange={e => setTask(e.target.value)} onKeyDown={e => e.key === "Enter" && add()} /><button onClick={add}>Add</button></div>
    {d.actions.length === 0 ? <div className="empty"><h3>No action items</h3><p className="muted">Tasks mentioned in the meeting are extracted automatically with the AI summary. Owners and due dates appear only when they were actually stated.</p></div>
      : d.actions.map(a => <ActionRow key={a.id + a.done} a={a} onChange={reload} onSeek={seek} />)}
  </div>;
}

export function ActionRow({ a, onChange, onSeek, meetingTitle, onOpen }: { a: ActionItem; onChange: () => void; onSeek?: (t: number) => void; meetingTitle?: string; onOpen?: () => void }) {
  const [x, setX] = useState(a); const [open, setOpen] = useState(false);
  const save = async (v: ActionItem) => { setX(v); await api.saveAction(v); onChange(); };
  return <div className="action">
    <div className="arow"><input type="checkbox" checked={x.done} onChange={e => save({ ...x, done: e.target.checked })} />
      <div className="grow"><div className={x.done ? "done" : ""}>{x.task}</div>
        <div className="muted small">👤 {x.owner || "No owner"} · 📅 {x.due || "No due date"}{x.priority && ` · ⚑ ${x.priority}`}
          {x.sourceTime != null && onSeek && <> · <a className="stamp" onClick={() => onSeek(x.sourceTime!)}>{clock(x.sourceTime)}</a></>}
          {meetingTitle && <> · <a onClick={onOpen}>{meetingTitle}</a></>}</div></div>
      <button className="icon" onClick={() => setOpen(!open)}>{open ? "▴" : "✎"}</button></div>
    {open && <div className="aedit">
      <input value={x.task} onChange={e => setX({ ...x, task: e.target.value })} placeholder="Task" />
      <input value={x.owner} onChange={e => setX({ ...x, owner: e.target.value })} placeholder="Owner (not specified)" />
      <input value={x.due} onChange={e => setX({ ...x, due: e.target.value })} placeholder="Due (not specified)" />
      <select value={x.priority} onChange={e => setX({ ...x, priority: e.target.value })}><option value="">No priority</option><option value="high">High</option><option value="medium">Medium</option><option value="low">Low</option></select>
      <input value={x.notes} onChange={e => setX({ ...x, notes: e.target.value })} placeholder="Notes" />
      <div><button className="danger" onClick={async () => { await api.deleteAction(x.id, x.meetingId); onChange(); }}>Delete</button><button className="primary" onClick={() => { save(x); setOpen(false); }}>Save</button></div>
    </div>}
  </div>;
}

function MindMapTab({ d, ctx, reload }: { d: Detail; ctx: Ctx; reload: () => void }) {
  const [busy, setBusy] = useState(false); const [err, setErr] = useState("");
  const gen = async () => { setBusy(true); setErr(""); try { await api.mindmap(d.meeting.id); reload(); } catch (e) { setErr(errText(e)); } setBusy(false); };
  return <div>
    <div className="toolbar"><button className="primary" disabled={busy || !d.segments.length} onClick={gen}>{d.mindmap ? "Regenerate" : "Generate Mind Map"}</button>
      {busy && <span className="muted small">Building on this computer…</span>}{err && <span className="errtext small">{err}</span>}</div>
    {d.mindmap ? <MindMap root={d.mindmap} name={d.meeting.title} /> : <div className="empty"><h3>Mind map</h3><p className="muted">Turn this meeting's notes into a visual map of topics, decisions and tasks. Click a branch to expand or collapse it; drag to pan; scroll to zoom.</p></div>}
  </div>;
}

function NotesTab({ d, reload, seek }: { d: Detail; reload: () => void; seek: (t: number) => void }) {
  const [notes, setNotes] = useState(d.meeting.userNotes);
  const dirty = notes !== d.meeting.userNotes;
  return <div className="notesgrid">
    <div><div className="toolbar"><b>My Notes</b><div className="grow" />{dirty && <button className="primary" onClick={async () => { await api.save({ ...d.meeting, userNotes: notes }); reload(); }}>Save</button>}</div>
      <p className="muted small">Notes typed during the recording, or add more now. They are given to the AI when it writes the summary.</p>
      <textarea className="editor tall" value={notes} onChange={e => setNotes(e.target.value)} /></div>
    <div><b>Highlights</b>
      {d.highlights.length === 0 && <p className="muted small">Use the Highlight button while recording to mark important moments.</p>}
      {d.highlights.map(h => <div key={h.id} className="hl"><a className="stamp" onClick={() => seek(h.time)}>{clock(h.time)}</a> {h.label}
        <a className="muted" onClick={async () => { await api.deleteHighlight(h.id); reload(); }}> ×</a></div>)}</div>
  </div>;
}

function ExportModal({ d, onClose, ctx }: { d: Detail; onClose: () => void; ctx: Ctx }) {
  const [content, setContent] = useState("summary"); const [format, setFormat] = useState("pdf");
  const m = d.meeting;
  const names: Record<string, string> = { tldr: "TLDR", summary: "Notes", transcript: "Transcript", actions: "Action Items", everything: "Notes and Transcript" };
  const go = async () => {
    try {
      if (format === "pdf") { const md = await api.exportMarkdown(m.id, content); const body = md.split("\n").slice(4).join("\n"); await printDoc(m.title, `${dateText(m.createdAt)} · ${duration(m.duration)}`, body); onClose(); return; }
      if (format === "audio") { const ext = m.audioFile.split(".").pop(); const p = await save({ defaultPath: `${m.title}.${ext}` }); if (p) { await api.exportAudio(m.id, p); onClose(); } return; }
      const p = await save({ defaultPath: `${m.title.replace(/[\\/:*?"<>|]/g, "-")} - ${names[content]}.${format}`, filters: [{ name: format.toUpperCase(), extensions: [format] }] });
      if (p) { await api.exportFile(m.id, content, format, p); ctx.toast("Exported."); onClose(); }
    } catch (e) { ctx.toast(errText(e)); }
  };
  return <div className="modalbg" onClick={onClose}><div className="modal" onClick={e => e.stopPropagation()}>
    <h3>Export “{m.title}”</h3>
    <label>Content <select value={content} onChange={e => setContent(e.target.value)}>
      <option value="tldr">TL;DR (one page)</option><option value="summary">AI Summary</option><option value="transcript">Full Transcript</option><option value="actions">Action Items</option><option value="everything">Summary + Transcript</option></select></label>
    <label>Format <select value={format} onChange={e => setFormat(e.target.value)}>
      <option value="pdf">PDF (print → Save as PDF)</option><option value="docx">Word (.docx)</option><option value="md">Markdown (.md)</option><option value="txt">Plain text (.txt)</option>
      <option value="srt">Subtitles (.srt)</option><option value="vtt">WebVTT (.vtt)</option><option value="audio">Original audio</option></select></label>
    {(format === "srt" || format === "vtt") && <p className="muted small">Subtitle files always contain the timed transcript.</p>}
    <div className="row-end"><button onClick={async () => { navigator.clipboard.writeText(await api.exportMarkdown(m.id, content)); ctx.toast("Copied."); }}>Copy to Clipboard</button><div className="grow" />
      <button onClick={onClose}>Cancel</button><button className="primary" onClick={go}>Export…</button></div>
  </div></div>;
}
