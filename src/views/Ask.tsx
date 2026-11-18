import { useEffect, useRef, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { api, listen, ActionItem, Chat, errText } from "../api";
import { Markdown } from "../components/Markdown";
import { ActionRow } from "./Detail";
import { Ctx } from "../App";

const MEETING_Q = ["What did we decide?", "What tasks were assigned, and to whom?", "What deadlines were mentioned?", "What questions did we leave unanswered?",
  "Summarize everything discussed about the budget.", "Write a follow-up email based on this meeting.", "Give me a list of all commitments made."];
const LIB_Q = ["What have we decided about the budget across meetings?", "Which action items mention volunteers?", "When did we last discuss the website?", "List all deadlines mentioned in recent meetings."];

export function AskView({ ctx, meetingId, onSeek }: { ctx: Ctx; meetingId: string | null; onSeek?: (t: number) => void }) {
  const [msgs, setMsgs] = useState<Chat[]>([]);
  const [input, setInput] = useState("");
  const [stream, setStream] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [hasLlm, setHasLlm] = useState(true);
  const end = useRef<HTMLDivElement>(null);
  useEffect(() => { api.chat(meetingId).then(setMsgs); api.models().then(m => setHasLlm(!!m.activeLlm)); }, [meetingId]);
  useEffect(() => { end.current?.scrollIntoView({ behavior: "smooth" }); }, [msgs, stream]);

  const send = async (q0?: string) => {
    const q = (q0 ?? input).trim(); if (!q || busy) return;
    setInput(""); setErr(""); setBusy(true); setStream("");
    setMsgs(m => [...m, { id: Date.now(), meetingId, role: "user", content: q, createdAt: Date.now() / 1000 }]);
    const un = await listen<string>("ask-token", e => setStream(s => s + e.payload));
    try { await api.ask(q, meetingId); } catch (e) { setErr(errText(e)); }
    un(); setStream(""); setBusy(false);
    api.chat(meetingId).then(setMsgs);
  };
  const exportAnswer = async (m: Chat, fmt: string) => {
    const q = [...msgs].reverse().find(x => x.role === "user" && x.id < m.id)?.content ?? "AI answer";
    const p = await save({ defaultPath: `${q.slice(0, 60).replace(/[\\/:*?"<>|]/g, "-")}.${fmt}` });
    if (p) await (await import("../api")).api.exportText(p, `# ${q}\n\n${m.content}`, fmt, q);
  };
  const saveNote = async (m: Chat) => {
    if (!meetingId) return;
    const d = await api.detail(meetingId);
    const q = [...msgs].reverse().find(x => x.role === "user" && x.id < m.id)?.content ?? "AI answer";
    await api.save({ ...d.meeting, userNotes: `${d.meeting.userNotes}\n\n### ${q}\n${m.content}`.trim() });
    ctx.toast("Saved to the meeting's Notes.");
  };

  if (!hasLlm) return <div className="empty"><h3>Ask AI needs a local language model</h3><p className="muted">Download or import one in Settings → AI Models. After that, questions are answered entirely offline.</p><button className="primary" onClick={ctx.showSetup}>Set Up AI Model…</button></div>;
  return <div className={`ask ${meetingId ? "" : "page-pad"}`}>
    {!meetingId && <><h1>Ask AI About All Meetings</h1><p className="muted">Searches every saved transcript, summary and note on this computer, then answers with the local model.</p></>}
    <div className="chat">
      {msgs.length === 0 && !busy && <div className="suggest"><b>Try asking:</b>{(meetingId ? MEETING_Q : LIB_Q).map(q => <button key={q} onClick={() => send(q)}>{q}</button>)}</div>}
      {msgs.map(m => m.role === "user" ? <div key={m.id} className="bubble user">{m.content}</div>
        : <div key={m.id} className="bubble ai"><Markdown md={m.content} onSeek={onSeek} />
            <div className="small links"><a onClick={() => navigator.clipboard.writeText(m.content)}>Copy</a>
              {meetingId && <a onClick={() => saveNote(m)}>Save as Note</a>}
              <a onClick={() => exportAnswer(m, "docx")}>Word</a><a onClick={() => exportAnswer(m, "md")}>Markdown</a></div></div>)}
      {busy && <div className="bubble ai">{stream ? <Markdown md={stream} onSeek={onSeek} /> : <span className="muted">Thinking on this computer…</span>}</div>}
      {err && <p className="errtext">{err}</p>}
      <div ref={end} />
    </div>
    <div className="composer">
      <textarea rows={2} placeholder={meetingId ? "Ask about this meeting…" : "Ask about any of your meetings…"} value={input} onChange={e => setInput(e.target.value)}
        onKeyDown={e => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); send(); } }} disabled={busy} />
      {busy ? <button onClick={() => api.askCancel()}>Stop</button> : <button className="primary" disabled={!input.trim()} onClick={() => send()}>Send</button>}
    </div>
    <div className="muted small">Answers use only your saved meetings and run on this computer. They can contain mistakes — follow the timestamps to check.
      {msgs.length > 0 && <a onClick={async () => { await api.clearChat(meetingId); setMsgs([]); }}> Clear conversation</a>}</div>
  </div>;
}

export function ActionsView({ ctx }: { ctx: Ctx }) {
  const [items, setItems] = useState<ActionItem[]>([]);
  const [showDone, setShowDone] = useState(false);
  const [owner, setOwner] = useState("");
  const load = () => api.allActions().then(setItems);
  useEffect(() => { load(); }, [ctx.meetings]);
  const titles = Object.fromEntries(ctx.meetings.map(m => [m.id, m.title]));
  const owners = [...new Set(items.map(i => i.owner).filter(Boolean))].sort();
  const vis = items.filter(i => (showDone || !i.done) && (!owner || i.owner === owner));
  return <div className="page-pad">
    <div className="toolbar"><h1>Action Items</h1><div className="grow" />
      <select value={owner} onChange={e => setOwner(e.target.value)}><option value="">Everyone</option>{owners.map(o => <option key={o}>{o}</option>)}</select>
      <label className="small"><input type="checkbox" checked={showDone} onChange={e => setShowDone(e.target.checked)} /> Show completed</label></div>
    {vis.length === 0 ? <div className="empty"><h3>{items.length ? "All caught up" : "No action items yet"}</h3><p className="muted">Action items from every meeting collect here.</p></div>
      : vis.map(a => <ActionRow key={a.id + a.done} a={a} onChange={() => { load(); ctx.reload(); }} meetingTitle={titles[a.meetingId] ?? "Meeting"} onOpen={() => ctx.open(a.meetingId)} />)}
  </div>;
}
