import { useCallback, useEffect, useMemo, useState } from "react";
import { api, listen, Meeting, Collection, Progress, Settings, SysInfo, duration, dateText, errText } from "./api";
import { MeetingDetail } from "./views/Detail";
import { RecordView, ImportView } from "./views/Record";
import { ActionsView, AskView } from "./views/Ask";
import { SettingsView, SetupWizard } from "./views/Settings";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { confirm } from "@tauri-apps/plugin-dialog";

export type Page = "record" | "import" | "all" | "favorites" | "actions" | "ask" | "archive" | "settings" | `col:${string}`;

export interface Ctx {
  meetings: Meeting[]; collections: Collection[]; progress: Progress; settings: Settings; sys: SysInfo | null;
  reload: () => Promise<void>; setSettings: (s: Settings) => void; open: (id: string) => void; go: (p: Page) => void;
  toast: (msg: string) => void; showSetup: () => void;
}

const STATUS: Record<string, string> = { recording: "Recording", recorded: "Not processed", queued: "Queued", preparing: "Preparing audio", transcribing: "Transcribing",
  summarizing: "Summarizing", ready: "Ready", transcribed: "Transcribed", failed: "Failed", interrupted: "Interrupted" };
export const statusLabel = (s: string) => STATUS[s] ?? s;

export default function App() {
  const [meetings, setMeetings] = useState<Meeting[]>([]);
  const [collections, setCollections] = useState<Collection[]>([]);
  const [openActions, setOpenActions] = useState(0);
  const [progress, setProgress] = useState<Progress>({ stage: "", fraction: 0, queued: [] });
  const [settings, setSettingsState] = useState<Settings | null>(null);
  const [sys, setSys] = useState<SysInfo | null>(null);
  const [page, setPage] = useState<Page>("all");
  const [selected, setSelected] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<Record<string, string>>({});
  const [sort, setSort] = useState("newest");
  const [toastMsg, setToast] = useState<string | null>(null);
  const [setup, setSetup] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [newCol, setNewCol] = useState<string | null>(null);

  const reload = useCallback(async () => {
    const r = await api.list(); setMeetings(r.meetings); setCollections(r.collections); setOpenActions(r.openActions);
  }, []);
  const toast = useCallback((m: string) => { setToast(m); setTimeout(() => setToast(null), 6000); }, []);

  useEffect(() => {
    reload(); api.progress().then(setProgress); api.systemInfo().then(setSys);
    api.settings().then(s => { setSettingsState(s); api.models().then(m => { if (!s.setupCompleted || !m.activeWhisper) setSetup(true); }); });
    const un = [listen("meetings-changed", () => reload()), listen<Progress>("progress", e => setProgress(e.payload))];
    const dd = getCurrentWebview().onDragDropEvent(e => {
      if (e.payload.type === "enter" || e.payload.type === "over") setDragging(true);
      else if (e.payload.type === "leave") setDragging(false);
      else if (e.payload.type === "drop") {
        setDragging(false);
        const media = e.payload.paths.filter(p => /\.(mp3|wav|m4a|aac|flac|ogg|mp4|mov|m4v|webm)$/i.test(p));
        if (media.length) api.importFiles(media).then(ids => { setPage("all"); setSelected(ids[0]); reload(); }).catch(err => toast(errText(err)));
        else toast("Drop audio (MP3, WAV, M4A, AAC, FLAC) or video (MP4, MOV) files.");
      }
    });
    return () => { un.forEach(p => p.then(f => f())); dd.then(f => f()); };
  }, [reload, toast]);

  useEffect(() => {
    if (!query.trim()) { setHits({}); return; }
    const t = setTimeout(() => api.search(query).then(r => { const h: Record<string, string> = {}; r.forEach(x => { if (!h[x.meetingId] || x.kind !== "title") h[x.meetingId] = x.snippet; }); setHits(h); }), 200);
    return () => clearTimeout(t);
  }, [query]);

  const setSettings = (s: Settings) => { setSettingsState(s); api.saveSettings(s); };
  const ctx: Ctx | null = settings ? { meetings, collections, progress, settings, sys, reload, setSettings, open: id => { setPage("all"); setSelected(id); },
    go: setPage, toast, showSetup: () => setSetup(true) } : null;

  const visible = useMemo(() => {
    let l = meetings.filter(m => page === "archive" ? m.isArchived : !m.isArchived);
    if (page === "favorites") l = l.filter(m => m.isFavorite);
    if (page.startsWith("col:")) l = l.filter(m => m.collectionId === page.slice(4));
    const q = query.trim().toLowerCase();
    if (q) l = l.filter(m => hits[m.id] !== undefined || m.title.toLowerCase().includes(q) || m.tags.some(t => t.toLowerCase().includes(q)) || dateText(m.createdAt).toLowerCase().includes(q));
    const s = [...l];
    if (sort === "oldest") s.sort((a, b) => a.createdAt - b.createdAt); else if (sort === "title") s.sort((a, b) => a.title.localeCompare(b.title));
    else if (sort === "longest") s.sort((a, b) => b.duration - a.duration); else s.sort((a, b) => b.createdAt - a.createdAt);
    return s;
  }, [meetings, page, query, hits, sort]);

  if (!ctx) return <div className="center muted">Loading…</div>;
  const isList = ["all", "favorites", "archive"].includes(page) || page.startsWith("col:");
  const nav = (p: Page, label: string, icon: string, badge?: number) =>
    <button className={`nav ${page === p ? "active" : ""}`} onClick={() => setPage(p)}><span className="ico">{icon}</span>{label}{badge ? <span className="badge">{badge}</span> : null}</button>;

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">OpenMeeting Notes</div>
        {nav("record", "New Recording", "●")}
        {nav("import", "Import Audio", "⤓")}
        <div className="section">Library</div>
        {nav("all", "All Meetings", "▤", meetings.filter(m => !m.isArchived).length)}
        {nav("favorites", "Favorites", "★")}
        {nav("actions", "Action Items", "☑", openActions)}
        {nav("ask", "Ask AI (All Meetings)", "✦")}
        {nav("archive", "Archive", "⌂")}
        <div className="section">Collections <button className="link" onClick={() => setNewCol("")}>＋</button></div>
        {newCol !== null && <input className="colin" autoFocus placeholder="Collection name" value={newCol} onChange={e => setNewCol(e.target.value)}
          onBlur={() => setNewCol(null)} onKeyDown={e => { if (e.key === "Enter" && newCol.trim()) { api.saveCollection({ id: crypto.randomUUID(), name: newCol.trim() }).then(reload); setNewCol(null); } if (e.key === "Escape") setNewCol(null); }} />}
        {collections.map(c => <div key={c.id} className="navrow">{nav(`col:${c.id}`, c.name, "▢")}
          <button className="link small" title="Delete collection" onClick={async () => { if (await confirm(`Delete collection “${c.name}”? Meetings are kept.`)) api.deleteCollection(c.id).then(reload); }}>×</button></div>)}
        <div className="grow" />
        {nav("settings", "Settings", "⚙")}
        <div className="local" title="Recording, transcription and AI all run on this computer. Nothing is uploaded.">🔒 On-device AI</div>
      </aside>
      <main className="main">
        {page === "record" && <RecordView ctx={ctx} />}
        {page === "import" && <ImportView ctx={ctx} />}
        {page === "actions" && <ActionsView ctx={ctx} />}
        {page === "ask" && <AskView ctx={ctx} meetingId={null} />}
        {page === "settings" && <SettingsView ctx={ctx} />}
        {isList && <div className="split">
          <div className="list">
            <div className="listhead">
              <input className="search" placeholder="Search titles, dates, words spoken, notes…" value={query} onChange={e => setQuery(e.target.value)} />
              <select value={sort} onChange={e => setSort(e.target.value)}><option value="newest">Newest</option><option value="oldest">Oldest</option><option value="title">Title</option><option value="longest">Longest</option></select>
            </div>
            {visible.length === 0 && <div className="empty small">{query ? "No matches. Search covers titles, dates, tags, transcripts, summaries, notes and action items." : "Recordings and imported files appear here."}</div>}
            {visible.map(m => <div key={m.id} className={`row ${selected === m.id ? "sel" : ""}`} onClick={() => setSelected(m.id)}>
              <div className="rowtitle">{m.isFavorite && <span className="star">★</span>}{m.title}</div>
              <div className="muted small">{dateText(m.createdAt)} · {duration(m.duration)}{m.source === "imported" ? " · imported" : ""}</div>
              <div className="rowmeta"><span className={`status s-${m.status}`}>{statusLabel(m.status)}</span>{m.tags.slice(0, 3).map(t => <span key={t} className="tag">#{t}</span>)}</div>
              {progress.meetingId === m.id && <div className="bar"><div style={{ width: `${progress.fraction * 100}%` }} /></div>}
              {hits[m.id] && <div className="snippet">{hits[m.id].replace(/[«»]/g, "")}</div>}
            </div>)}
          </div>
          <div className="detail">
            {selected && meetings.some(m => m.id === selected)
              ? <MeetingDetail key={selected} id={selected} ctx={ctx} onDeleted={() => setSelected(null)} />
              : <div className="empty">
                  <h2>{meetings.length ? "Select a meeting" : "Record your first meeting"}</h2>
                  <p className="muted">{meetings.length ? "Choose a meeting to see its TL;DR, summary, transcript, action items and more." : "Click New Recording, hold your meeting, then Stop. Everything is transcribed and summarized on this computer."}</p>
                  {!meetings.length && <button className="primary big" onClick={() => setPage("record")}>New Recording</button>}
                </div>}
          </div>
        </div>}
      </main>
      {setup && <SetupWizard ctx={ctx} onClose={() => setSetup(false)} />}
      {dragging && <div className="dropzone">Drop audio or video files to import</div>}
      {toastMsg && <div className="toast" onClick={() => setToast(null)}>{toastMsg}</div>}
    </div>
  );
}
