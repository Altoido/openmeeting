import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface Meeting {
  id: string; title: string; createdAt: number; duration: number; audioFile: string; originalFileName?: string | null;
  source: string; status: string; statusDetail: string; progress: number; language: string; collectionId?: string | null;
  isFavorite: boolean; isArchived: boolean; userNotes: string; templateId: string; stereoSources: boolean;
  transcribedUntil: number; titleUserSet: boolean; tags: string[];
}
export interface Segment { id: number; index: number; start: number; end: number; text: string; originalText: string; speaker?: string | null }
export interface ActionItem { id: string; meetingId: string; task: string; owner: string; due: string; priority: string; done: boolean; notes: string; sourceTime?: number | null }
export interface Summary { id: number; templateId: string; templateName: string; markdown: string; structuredJson: string; modelName: string; createdAt: number }
export interface Chat { id: number; meetingId?: string | null; role: string; content: string; createdAt: number }
export interface Highlight { id: number; time: number; label: string }
export interface Collection { id: string; name: string }
export interface MindNode { label: string; children: MindNode[] }
export interface Detail {
  meeting: Meeting; audioPath: string | null; segments: Segment[]; speakers: Record<string, string>; summaries: Summary[];
  tldr: { markdown: string; model: string; createdAt: number } | null; actions: ActionItem[]; highlights: Highlight[]; mindmap: MindNode | null; chat: Chat[];
}
export interface Template { id: string; name: string; icon: string; description: string; instructions: string }
export interface Progress { meetingId?: string | null; stage: string; fraction: number; queued: string[] }
export interface RecStatus { active: boolean; paused: boolean; elapsed: number; micLevel: number; systemLevel: number; warning?: string | null; meetingId?: string | null }
export interface Settings {
  whisperModel: string; llmModel: string; useVad: boolean; language: string; autoTranscribe: boolean; autoSummarize: boolean; autoTitle: boolean;
  defaultTemplate: string; recordingSource: string; inputDevice: string; llmIdleMinutes: number; setupCompleted: boolean; consentDismissed: boolean;
  checkUpdates: boolean; lastUpdateCheck: number; skippedVersion: string;
}
export interface Update { version: string; current: string; url: string; notes: string }
export interface ModelSpec { id: string; kind: string; name: string; fileName: string; url: string; approxBytes: number; sha1?: string; license: string; summary: string; context: number; minRamGb: number }
export interface ModelsState { catalog: ModelSpec[]; installed: Record<string, { file: string; bytes: number }[]>; activeWhisper?: string | null; activeLlm?: string | null }
export interface SysInfo {
  info: { os: string; arch: string; memoryGb: number; cpus: number; appleSilicon: boolean; freeDiskGb: number; gpu: boolean };
  recommendedWhisper: string; recommendedLlm: string; systemAudio: boolean; engines: { whisper: boolean; llama: boolean }; dataDir: string; libraryDir: string; modelsDir: string;
}

export const api = {
  systemInfo: () => invoke<SysInfo>("system_info"),
  settings: () => invoke<Settings>("settings_get"),
  saveSettings: (s: Settings) => invoke("settings_set", { s }),
  list: () => invoke<{ meetings: Meeting[]; collections: Collection[]; openActions: number }>("meetings_list"),
  detail: (id: string) => invoke<Detail>("meeting_detail", { id }),
  save: (m: Meeting) => invoke("meeting_save", { m }),
  remove: (id: string) => invoke("meeting_delete", { id }),
  search: (q: string) => invoke<{ meetingId: string; kind: string; ref: string; snippet: string }[]>("search", { q }),
  importFiles: (paths: string[]) => invoke<string[]>("import_files", { files: paths }),
  devices: () => invoke<string[]>("input_devices"),
  recStart: (title: string, source: string, device: string | null) => invoke<string>("record_start", { title, source, device }),
  recStatus: () => invoke<RecStatus>("record_status"),
  recPause: (paused: boolean) => invoke("record_pause", { paused }),
  recStop: (notes: string, highlights: [number, string][]) => invoke<string>("record_stop", { notes, highlights }),
  process: (id: string, kind: "full" | "transcribe" | "summarize", templateId?: string) => invoke("process", { id, kind, templateId }),
  cancel: (id: string) => invoke("process_cancel", { id }),
  progress: () => invoke<Progress>("progress_get"),
  segment: (meetingId: string, segId: number, o: { text?: string; speaker?: string | null; setSpeaker?: boolean; revert?: boolean }) =>
    invoke("segment_update", { meetingId, segId, text: o.text ?? null, speaker: o.speaker ?? null, setSpeaker: !!o.setSpeaker, revert: !!o.revert }),
  speaker: (meetingId: string, key: string, name: string) => invoke("speaker_set", { meetingId, key, name }),
  addHighlight: (meetingId: string, time: number, label: string) => invoke("highlight_add", { meetingId, time, label }),
  deleteHighlight: (id: number) => invoke("highlight_delete", { id }),
  updateSummary: (meetingId: string, id: number, markdown: string) => invoke("summary_update", { meetingId, id, markdown }),
  saveTldr: (meetingId: string, markdown: string, model: string) => invoke("tldr_save", { meetingId, markdown, model }),
  saveAction: (a: ActionItem) => invoke("action_save", { a }),
  deleteAction: (id: string, meetingId: string) => invoke("action_delete", { id, meetingId }),
  allActions: () => invoke<ActionItem[]>("actions_all"),
  templates: () => invoke<Template[]>("templates_list"),
  customTemplates: () => invoke<[string, string, string][]>("custom_templates"),
  saveTemplate: (id: string, name: string, instructions: string) => invoke("template_save", { id, name, instructions }),
  deleteTemplate: (id: string) => invoke("template_delete", { id }),
  saveCollection: (c: Collection) => invoke("collection_save", { c }),
  deleteCollection: (id: string) => invoke("collection_delete", { id }),
  ask: (question: string, meetingId: string | null) => invoke<string>("ask", { question, meetingId }),
  askCancel: () => invoke("ask_cancel"),
  chat: (meetingId: string | null) => invoke<Chat[]>("chat_get", { meetingId }),
  clearChat: (meetingId: string | null) => invoke("chat_clear", { meetingId }),
  mindmap: (id: string) => invoke<MindNode>("mindmap_generate", { id }),
  tldr: (id: string) => invoke<string>("tldr_generate", { id }),
  exportFile: (id: string, content: string, format: string, path: string) => invoke("export_file", { id, content, format, path }),
  exportMarkdown: (id: string, content: string) => invoke<string>("export_markdown", { id, content }),
  exportText: (path: string, markdown: string, format: string, title: string) => invoke("export_text", { path, markdown, format, title }),
  exportAudio: (id: string, path: string) => invoke("export_audio", { id, path }),
  peaks: (id: string) => invoke<number[]>("peaks", { id }),
  models: () => invoke<ModelsState>("models_state"),
  download: (id: string, url?: string) => invoke("model_download", { id, url: url ?? null }),
  cancelDownload: (id: string) => invoke("model_cancel", { id }),
  deleteModel: (kind: string, file: string) => invoke("model_delete", { kind, file }),
  importModel: (path: string, kind: string) => invoke<string>("model_import", { path, kind }),
  llmTest: () => invoke<string>("llm_test"),
  checkUpdate: (manual: boolean) => invoke<Update | null>("check_update", { manual }),
  version: () => invoke<string>("app_version"),
};
export { listen };

export const clock = (s: number) => { const t = Math.max(0, Math.floor(s)); const h = Math.floor(t / 3600), m = Math.floor((t % 3600) / 60), x = t % 60;
  return h ? `${h}:${String(m).padStart(2, "0")}:${String(x).padStart(2, "0")}` : `${String(m).padStart(2, "0")}:${String(x).padStart(2, "0")}`; };
export const duration = (s: number) => { const t = Math.round(s); if (t < 60) return `${t} sec`; const h = Math.floor(t / 3600), m = Math.floor((t % 3600) / 60); return h ? `${h} hr ${m} min` : `${m} min`; };
export const dateText = (ts: number) => new Date(ts * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
export const bytes = (n: number) => n >= 1e9 ? `${(n / 1e9).toFixed(1)} GB` : `${Math.round(n / 1e6)} MB`;
export const speakerName = (key: string | null | undefined, names: Record<string, string>) => {
  if (!key) return null; if (names[key]?.trim()) return names[key];
  if (key === "ME") return "Me"; if (key === "REMOTE") return "Remote participants";
  return /^S\d+$/.test(key) ? `Speaker ${key.slice(1)}` : key;
};
export const errText = (e: unknown) => typeof e === "string" ? e : (e as Error)?.message ?? String(e);
