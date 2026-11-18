import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, listen, ModelsState, ModelSpec, Template, bytes, errText } from "../api";
import { Ctx } from "../App";

type DL = Record<string, { received: number; total: number }>;

export function ModelsPanel({ ctx }: { ctx: Ctx }) {
  const [st, setSt] = useState<ModelsState | null>(null);
  const [dl, setDl] = useState<DL>({});
  const [err, setErr] = useState("");
  const [url, setUrl] = useState("");
  const [test, setTest] = useState("");
  const load = () => api.models().then(setSt);
  useEffect(() => {
    load();
    const un = listen<{ id: string; received?: number; total?: number; done?: boolean }>("download-progress", e => {
      const p = e.payload;
      setDl(d => { const n = { ...d }; if (p.done) delete n[p.id]; else n[p.id] = { received: p.received ?? 0, total: p.total ?? 0 }; return n; });
      if (p.done) load();
    });
    return () => { un.then(f => f()); };
  }, []);
  if (!st || !ctx.sys) return null;
  const s = ctx.settings; const mem = ctx.sys.info.memoryGb;
  const isInstalled = (m: ModelSpec) => st.installed[m.kind]?.some(f => f.file === m.fileName);
  const start = async (id: string, u?: string) => {
    setErr(""); setDl(d => ({ ...d, [id]: { received: 0, total: 0 } }));
    try { await api.download(id, u); } catch (e) { if (errText(e) !== "cancelled") setErr(errText(e)); }
    setDl(d => { const n = { ...d }; delete n[id]; return n; }); load();
  };
  const use = (m: ModelSpec) => ctx.setSettings({ ...s, ...(m.kind === "llm" ? { llmModel: m.fileName } : { whisperModel: m.fileName }) });
  const active = (m: ModelSpec) => m.kind === "llm" ? st.activeLlm === m.fileName : st.activeWhisper === m.fileName;
  const row = (m: ModelSpec, rec: boolean) => <div key={m.id} className="mrow">
    <div className="grow"><b>{m.name}</b>{rec && <span className="pill">Recommended</span>}
      <div className="muted small">{m.summary} · {bytes(m.approxBytes)} · {m.license}</div>
      {m.kind === "llm" && m.minRamGb > Math.round(mem) && <div className="warntext small">Needs about {m.minRamGb} GB of memory — likely too large for this computer.</div>}</div>
    {dl[m.id] ? <div className="dl"><div className="bar"><div style={{ width: `${dl[m.id].total ? dl[m.id].received / dl[m.id].total * 100 : 2}%` }} /></div>
        <span className="small">{bytes(dl[m.id].received)}{dl[m.id].total ? ` of ${bytes(dl[m.id].total)}` : ""}</span><button onClick={() => api.cancelDownload(m.id)}>×</button></div>
      : isInstalled(m) ? <>{m.kind !== "vad" && <button disabled={active(m)} className={active(m) ? "ok" : ""} onClick={() => use(m)}>{active(m) ? "✓ In use" : "Use"}</button>}
          <button title="Remove" onClick={async () => { await api.deleteModel(m.kind, m.fileName); load(); }}>🗑</button></>
      : <button onClick={() => start(m.id)}>Download {bytes(m.approxBytes)}</button>}
  </div>;
  const imp = async (kind: string) => {
    const p = await open({ multiple: false }); if (!p) return;
    try { await api.importModel(p as string, kind); load(); } catch (e) { setErr(errText(e)); }
  };
  const extra = [...(st.installed.whisper ?? []), ...(st.installed.llm ?? [])].filter(f => !st.catalog.some(c => c.fileName === f.file));
  return <div className="models">
    <div className="card"><b>This computer</b><div className="muted">{ctx.sys.info.os} · {ctx.sys.info.arch} · {Math.round(mem)} GB memory · {Math.round(ctx.sys.info.freeDiskGb)} GB free</div>
      <div className="small">{ctx.sys.engines.whisper ? "✓ Transcription engine ready" : "✗ Transcription engine missing"} · {ctx.sys.engines.llama ? "✓ AI engine ready" : "✗ AI engine missing"}
        {ctx.sys.info.gpu ? " · GPU acceleration (Metal)" : " · AI runs on the processor (slower than on Apple Silicon)"}</div></div>
    {err && <p className="errtext">{err}</p>}
    <div className="card"><h3>Speech-to-text</h3><p className="muted small">Whisper models turn speech into text. Larger models are more accurate and slower.</p>
      {st.catalog.filter(m => m.kind === "whisper").map(m => row(m, m.id === ctx.sys!.recommendedWhisper))}</div>
    <div className="card"><h3>AI language model</h3><p className="muted small">Writes summaries, TL;DRs, answers, titles and mind maps — on this computer.</p>
      {st.catalog.filter(m => m.kind === "llm").map(m => row(m, m.id === ctx.sys!.recommendedLlm))}
      <div className="toolbar"><input className="grow" placeholder="Other GGUF model: https://huggingface.co/…/resolve/main/model.gguf" value={url} onChange={e => setUrl(e.target.value)} />
        <button disabled={!url.trim()} onClick={() => start(`custom-${url.split("/").pop()}`, url.trim())}>Download</button></div></div>
    <div className="card"><h3>Silence detection</h3><label className="small"><input type="checkbox" checked={s.useVad} onChange={e => ctx.setSettings({ ...s, useVad: e.target.checked })} /> Use when installed</label>
      {st.catalog.filter(m => m.kind === "vad").map(m => row(m, false))}</div>
    <div className="card"><h3>Offline installation</h3><p className="muted small">On a computer without internet, copy model files over and import them. Whisper models are “ggml-….bin”; language models are “.gguf”.</p>
      <button onClick={() => imp("whisper")}>Import Whisper Model…</button> <button onClick={() => imp("llm")}>Import Language Model…</button> <button onClick={() => imp("vad")}>Import VAD Model…</button>
      {extra.map(f => <div key={f.file} className="mrow"><div className="grow">{f.file} <span className="muted small">{bytes(f.bytes)}</span></div>
        <button onClick={() => ctx.setSettings({ ...s, ...(f.file.endsWith(".gguf") ? { llmModel: f.file } : { whisperModel: f.file }) })}>Use</button>
        <button onClick={async () => { await api.deleteModel(f.file.endsWith(".gguf") ? "llm" : "whisper", f.file); load(); }}>🗑</button></div>)}</div>
    <div className="card"><h3>Check the AI model</h3><button disabled={!st.activeLlm || test === "…"} onClick={async () => { setTest("…"); try { setTest(await api.llmTest()); } catch (e) { setTest("✗ " + errText(e)); } }}>Run Quick Test</button> <span className="small">{test}</span></div>
  </div>;
}

export function SettingsView({ ctx }: { ctx: Ctx }) {
  const [tab, setTab] = useState("AI Models");
  const [tpls, setTpls] = useState<Template[]>([]);
  const [custom, setCustom] = useState<[string, string, string][]>([]);
  const [editing, setEditing] = useState<[string, string, string] | null>(null);
  const s = ctx.settings;
  const loadT = () => { api.templates().then(setTpls); api.customTemplates().then(setCustom); };
  useEffect(loadT, []);
  return <div className="page-pad">
    <h1>Settings</h1>
    <div className="tabs">{["AI Models", "Processing", "Templates", "Privacy & Storage", "About"].map(t => <button key={t} className={tab === t ? "active" : ""} onClick={() => setTab(t)}>{t}</button>)}</div>
    {tab === "AI Models" && <ModelsPanel ctx={ctx} />}
    {tab === "Processing" && <div className="card form">
      <label><input type="checkbox" checked={s.autoTranscribe} onChange={e => ctx.setSettings({ ...s, autoTranscribe: e.target.checked })} /> Transcribe automatically after recording or import</label>
      <label><input type="checkbox" checked={s.autoSummarize} onChange={e => ctx.setSettings({ ...s, autoSummarize: e.target.checked })} /> Create the AI summary and TL;DR automatically</label>
      <label><input type="checkbox" checked={s.autoTitle} onChange={e => ctx.setSettings({ ...s, autoTitle: e.target.checked })} /> Let AI suggest a meeting title (unless you typed one)</label>
      <label>Spoken language <select value={s.language} onChange={e => ctx.setSettings({ ...s, language: e.target.value })}>
        {[["auto", "Detect automatically"], ["en", "English"], ["es", "Spanish"], ["fr", "French"], ["de", "German"], ["pt", "Portuguese"], ["it", "Italian"], ["nl", "Dutch"], ["ko", "Korean"], ["zh", "Chinese"], ["ja", "Japanese"], ["ru", "Russian"], ["uk", "Ukrainian"], ["ar", "Arabic"], ["hi", "Hindi"], ["vi", "Vietnamese"], ["tl", "Tagalog"], ["sw", "Swahili"]].map(([c, n]) => <option key={c} value={c}>{n}</option>)}</select></label>
      <label>Default summary template <select value={s.defaultTemplate} onChange={e => ctx.setSettings({ ...s, defaultTemplate: e.target.value })}>{tpls.map(t => <option key={t.id} value={t.id}>{t.name}</option>)}</select></label>
      <label>Unload the AI model after <input type="number" min={1} max={120} value={s.llmIdleMinutes} onChange={e => ctx.setSettings({ ...s, llmIdleMinutes: Number(e.target.value) || 10 })} style={{ width: 60 }} /> idle minutes</label>
    </div>}
    {tab === "Templates" && <div className="card">
      <h3>Built-in templates</h3>{tpls.filter(t => !t.id.startsWith("custom-")).map(t => <div key={t.id} className="small"><b>{t.name}</b> — <span className="muted">{t.description}</span></div>)}
      <div className="toolbar"><h3>Custom templates</h3><div className="grow" /><button onClick={() => setEditing([`custom-${crypto.randomUUID()}`, "", ""])}>New Template</button></div>
      {custom.length === 0 && <p className="muted small">Describe the notes you want — for example: “Sections: Prayer requests, Volunteer needs, Budget questions. Keep it under one page.”</p>}
      {custom.map(c => <div key={c[0]} className="mrow"><div className="grow"><b>{c[1]}</b><div className="muted small">{c[2].slice(0, 140)}</div></div>
        <button onClick={() => setEditing(c)}>Edit</button><button onClick={async () => { await api.deleteTemplate(c[0]); loadT(); }}>🗑</button></div>)}
      {editing && <div className="modalbg"><div className="modal">
        <h3>Custom Template</h3>
        <input placeholder="Template name" value={editing[1]} onChange={e => setEditing([editing[0], e.target.value, editing[2]])} />
        <p className="muted small">What should the notes contain? Describe sections, tone and length in plain language.</p>
        <textarea className="editor" rows={8} value={editing[2]} onChange={e => setEditing([editing[0], editing[1], e.target.value])} />
        <div className="row-end"><div className="grow" /><button onClick={() => setEditing(null)}>Cancel</button>
          <button className="primary" disabled={!editing[1].trim() || !editing[2].trim()} onClick={async () => { await api.saveTemplate(...editing); setEditing(null); loadT(); }}>Save</button></div>
      </div></div>}
    </div>}
    {tab === "Privacy & Storage" && <div className="card">
      <p>No account, subscription, telemetry, analytics or ads. Recordings, transcripts and notes stay on this computer. Transcription and AI run locally. The internet is used only when you click Download for a model, and — if turned on in About — to check whether a new version of the app exists. The AI engine listens only on this computer (127.0.0.1) with a private key.</p>
      <p className="small">Library: <code>{ctx.sys?.libraryDir}</code></p><p className="small">Models: <code>{ctx.sys?.modelsDir}</code></p>
      <button onClick={() => ctx.sys && invoke("plugin:opener|reveal_item_in_dir", { path: ctx.sys.libraryDir })}>Show Library Folder</button>{" "}
      <button onClick={() => ctx.sys && invoke("plugin:opener|reveal_item_in_dir", { path: ctx.sys.modelsDir })}>Show Models Folder</button>
      <p className="muted small">Delete a meeting from its ⋯ menu. To delete everything, quit the app and delete the Library folder.</p>
    </div>}
    {tab === "About" && <About ctx={ctx} />}
    {tab === "About" && <div className="card"><p>Free, offline meeting recorder, transcriber and AI note-taker for macOS and Windows.</p>
      <p className="small">Built with whisper.cpp and llama.cpp (MIT License, the ggml authors), Tauri (MIT/Apache-2.0), Symphonia (MPL-2.0), cpal (Apache-2.0). Models are downloaded separately under their own licenses: Whisper (MIT), Silero VAD (MIT), Qwen3 (Apache 2.0).</p></div>}
  </div>;
}

function About({ ctx }: { ctx: Ctx }) {
  const [v, setV] = useState(""); const [msg, setMsg] = useState("");
  useEffect(() => { api.version().then(setV); }, []);
  const s = ctx.settings;
  return <div className="card"><h3>OpenMeeting Notes {v}</h3>
    <label className="small"><input type="checkbox" checked={s.checkUpdates} onChange={e => ctx.setSettings({ ...s, checkUpdates: e.target.checked })} /> Tell me when a new version is available</label>
    <p className="muted small">Checks this app's public download page about twice a day. Nothing about you or your meetings is sent, and nothing is installed without you.</p>
    <button onClick={async () => { setMsg("Checking…"); try { const u = await api.checkUpdate(true);
      if (u) { setMsg(`Version ${u.version} is available.`); openUrl(u.url); } else setMsg("You have the latest version."); } catch (e) { setMsg(errText(e)); } }}>Check for Updates</button> <span className="small">{msg}</span>
  </div>;
}

export function SetupWizard({ ctx, onClose }: { ctx: Ctx; onClose: () => void }) {
  const [step, setStep] = useState(0);
  const [st, setSt] = useState<ModelsState | null>(null);
  useEffect(() => { api.models().then(setSt); }, [step]);
  const sys = ctx.sys;
  const finish = () => { ctx.setSettings({ ...ctx.settings, setupCompleted: true }); onClose(); };
  const rec = st?.catalog.filter(m => sys && (m.id === sys.recommendedWhisper || m.id === sys.recommendedLlm || m.kind === "vad")) ?? [];
  return <div className="modalbg"><div className="modal wide">
    <div className="steps">{[0, 1, 2, 3].map(i => <div key={i} className={i <= step ? "on" : ""} />)}</div>
    {step === 0 && <><h2>Welcome to OpenMeeting Notes</h2><p>Record a meeting, click Stop, and get a transcript, a one-page TL;DR, a detailed summary, decisions, action items and answers to your questions.</p>
      <ul><li>Everything runs on this computer — nothing is uploaded.</li><li>Free: no account, subscription, quotas or usage limits.</li><li>Works offline once the AI models are installed (one-time download).</li><li>Recording is never secret: an indicator is always visible.</li></ul></>}
    {step === 1 && sys && <><h2>Your computer</h2><p>{sys.info.os} · {sys.info.arch} · {Math.round(sys.info.memoryGb)} GB memory · {Math.round(sys.info.freeDiskGb)} GB free disk</p>
      <p>{sys.engines.whisper && sys.engines.llama ? "✓ Local AI engines included" : "✗ Local AI engines are missing from this build"}</p>
      <h4>Recommended for this computer</h4><ul>{rec.map(m => <li key={m.id}>{m.name} — {bytes(m.approxBytes)}</li>)}</ul>
      <p className="muted">{sys.info.gpu ? "Apple Silicon detected: transcription and AI use the GPU for speed." : "AI runs on the processor here, so processing takes longer than on an Apple Silicon Mac. The recommended models are sized for this computer."}</p></>}
    {step === 2 && <><h2>Install AI models</h2><p className="muted">Downloaded once from Hugging Face, verified, and stored on this computer. Change or remove them later in Settings → AI Models.</p>
      <button className="primary" onClick={() => rec.filter(m => !st?.installed[m.kind]?.some(f => f.file === m.fileName)).forEach(m => api.download(m.id).then(() => api.models().then(setSt)).catch(e => ctx.toast(errText(e))))}>
        Download Recommended Models ({bytes(rec.filter(m => m.kind !== "vad").reduce((a, m) => a + m.approxBytes, 0))})</button>
      <div className="scroll"><ModelsPanel ctx={ctx} /></div></>}
    {step === 3 && <><h2>Permissions</h2><p>The first time you record, your system asks for permission to use the microphone — choose <b>Allow</b>.</p>
      <p className="muted">On Windows, computer audio (Zoom, Teams, Meet) is recorded directly from the speakers' output, with no extra permission. Downloads continue in the background; you can start recording now.</p></>}
    <div className="row-end">{step > 0 && <button onClick={() => setStep(step - 1)}>Back</button>}<div className="grow" />
      {step < 3 ? <><button className="link" onClick={finish}>Skip setup</button><button className="primary" onClick={() => setStep(step + 1)}>Continue</button></>
        : <button className="primary" onClick={finish}>Start Using OpenMeeting Notes</button>}</div>
  </div></div>;
}
