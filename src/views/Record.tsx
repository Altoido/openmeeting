import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, RecStatus, clock, errText } from "../api";
import { Ctx } from "../App";

const SOURCES = [
  { id: "microphone", label: "Microphone only", help: "In-person meetings, voice memos, external microphones." },
  { id: "system", label: "Computer audio only", help: "Audio playing on this computer (Zoom, Meet, Teams, videos) — without your voice." },
  { id: "both", label: "Microphone + computer audio", help: "Online meetings: your voice and the other participants, labeled “Me” and “Remote participants”." },
];

export function RecordView({ ctx }: { ctx: Ctx }) {
  const [st, setSt] = useState<RecStatus>({ active: false, paused: false, elapsed: 0, micLevel: 0, systemLevel: 0 });
  const [devices, setDevices] = useState<string[]>([]);
  const [title, setTitle] = useState("");
  const [notes, setNotes] = useState("");
  const [label, setLabel] = useState("");
  const [hls, setHls] = useState<[number, string][]>([]);
  const [consent, setConsent] = useState(false);
  const [saving, setSaving] = useState(false);
  const [bars, setBars] = useState<number[]>(Array(48).fill(0));
  const s = ctx.settings;
  const timer = useRef<number>();

  useEffect(() => {
    api.devices().then(setDevices);
    const tick = () => api.recStatus().then(x => { setSt(x); setBars(b => [...b.slice(1), Math.max(x.micLevel, x.systemLevel)]); });
    tick(); timer.current = window.setInterval(tick, 150);
    return () => clearInterval(timer.current);
  }, []);

  const start = async () => {
    setConsent(false);
    try { await api.recStart(title.trim() || `Recording — ${new Date().toLocaleString()}`, s.recordingSource, s.inputDevice || null); setNotes(""); setHls([]); }
    catch (e) { ctx.toast(errText(e)); }
  };
  const stop = async () => {
    setSaving(true);
    try { const id = await api.recStop(notes, hls); setTitle(""); ctx.open(id); }
    catch (e) { ctx.toast(errText(e)); }
    setSaving(false);
  };
  const sysOk = ctx.sys?.systemAudio;

  if (saving) return <div className="page-pad"><h1>Saving the recording…</h1></div>;
  if (st.active) return <div className="page-pad narrow">
    <div className="card center">
      <div className={`recdot ${st.paused ? "paused" : ""}`}>{st.paused ? "PAUSED" : "● RECORDING"}</div>
      <div className="bigtime">{clock(st.elapsed)}</div>
      <div className="bars">{bars.map((b, i) => <div key={i} style={{ height: `${Math.max(4, b * 100)}%` }} />)}</div>
      {s.recordingSource !== "system" && <Meter label="Microphone" v={st.micLevel} />}
      {s.recordingSource !== "microphone" && <Meter label="Computer audio" v={st.systemLevel} />}
      <div className="row-center">
        <button className="big" onClick={() => api.recPause(!st.paused)}>{st.paused ? "▶ Resume" : "❚❚ Pause"}</button>
        <button className="big stop" onClick={stop}>■ Stop & Save</button>
      </div>
      {st.warning && <p className="warntext">{st.warning}</p>}
      <p className="muted small">The computer is kept awake while recording. On a laptop, keep the lid open and plug it in for long meetings.</p>
    </div>
    <div className="card">
      <div className="toolbar"><b>Notes</b><div className="grow" />
        <input placeholder="Highlight label (optional)" value={label} onChange={e => setLabel(e.target.value)} />
        <button disabled={st.paused} onClick={() => { setHls([...hls, [st.elapsed, label.trim() || "Highlight"]]); setLabel(""); }}>★ Highlight</button></div>
      <textarea className="editor" rows={8} value={notes} onChange={e => setNotes(e.target.value)} placeholder="Type notes during the meeting. They're saved with it and given to the AI for the summary." />
      {hls.map(([t, l], i) => <div key={i} className="small">★ {clock(t)} {l}</div>)}
    </div>
  </div>;

  return <div className="page-pad narrow">
    <h1>New Recording</h1>
    <div className="card">
      <input className="bigin" placeholder="Meeting title (optional — AI can suggest one)" value={title} onChange={e => setTitle(e.target.value)} />
      <h4>What do you want to record?</h4>
      {SOURCES.map(x => <label key={x.id} className={`choice ${x.id !== "microphone" && !sysOk ? "disabled" : ""}`}>
        <input type="radio" disabled={x.id !== "microphone" && !sysOk} checked={s.recordingSource === x.id} onChange={() => ctx.setSettings({ ...s, recordingSource: x.id })} />
        <div><b>{x.label}</b><div className="muted small">{x.id !== "microphone" && !sysOk ? "Not available on this system in this version yet." : x.help}</div></div></label>)}
      {s.recordingSource !== "system" && <label>Microphone <select value={s.inputDevice} onChange={e => ctx.setSettings({ ...s, inputDevice: e.target.value })}>
        <option value="">System default</option>{devices.map(d => <option key={d}>{d}</option>)}</select></label>}
    </div>
    <div className="center">
      <button className="recbtn" title="Start recording" onClick={() => s.consentDismissed ? start() : setConsent(true)}><span /></button>
      <div className="big">Record</div>
      <p className="muted small">A recording indicator stays visible while recording. Please make sure everyone knows they are being recorded.</p>
    </div>
    {consent && <div className="modalbg"><div className="modal">
      <h3>Recording consent</h3>
      <p>Many places require that everyone in a conversation agrees to be recorded. Please tell participants you are recording and follow the laws and policies that apply to you.</p>
      <p className="muted">OpenMeeting Notes never records secretly, and everything stays on this computer.</p>
      <label><input type="checkbox" onChange={e => ctx.setSettings({ ...s, consentDismissed: e.target.checked })} /> Don't show this again</label>
      <div className="row-end"><div className="grow" /><button onClick={() => setConsent(false)}>Cancel</button><button className="primary" onClick={start}>Start Recording</button></div>
    </div></div>}
  </div>;
}

function Meter({ label, v }: { label: string; v: number }) {
  return <div className="meter"><span>{label}</span><div><div style={{ width: `${v * 100}%` }} /></div></div>;
}

export function ImportView({ ctx }: { ctx: Ctx }) {
  const pick = async () => {
    const r = await open({ multiple: true, filters: [{ name: "Audio or video", extensions: ["mp3", "wav", "m4a", "aac", "flac", "ogg", "mp4", "mov", "m4v", "webm"] }] });
    const paths = Array.isArray(r) ? r : r ? [r] : [];
    if (!paths.length) return;
    try { const ids = await api.importFiles(paths as string[]); ctx.open(ids[0]); } catch (e) { ctx.toast(errText(e)); }
  };
  return <div className="page-pad narrow center">
    <h1>Import Audio or Video</h1>
    <p className="muted">MP3, WAV, M4A, AAC, FLAC, OGG, MP4 and MOV. Video is converted to audio on this computer. Import several files at once — each becomes a meeting and is transcribed in turn.</p>
    <div className="dropbox"><div>Drag files anywhere onto this window</div><button className="primary big" onClick={pick}>Choose Files…</button></div>
  </div>;
}
