# Progress — OpenMeeting Notes (cross-platform, Tauri)

Replaces the macOS-only SwiftUI app (kept as reference in the earlier OpenMeetingNotes project). Decisions (2026-11-18): audience = team/church staff (unsigned installers OK), one Tauri codebase for Mac + Windows, public GitHub repo with Actions building installers.

## Verified in the Linux sandbox
- Rust core compiles; 7 unit tests pass (DB+FTS5, decoding/resampling, recorder crash recovery, markdown repair, TL;DR limit, retention/sanitize, whisper JSON, retrieval windows).
- Live test (`live_pipeline`, real llama-server + Qwen3-0.6B stand-in): summary, TL;DR, title, streaming Ask AI, mind map, DOCX — pass.
- Full app builds and runs (Xvfb): library, TL;DR tab, Ask AI streaming through the GUI confirmed by screenshots.
- TypeScript type-checks clean.

## Not yet verified
- Mac/Windows CI builds (workflow written, never run); Windows WASAPI loopback; microphone capture on real hardware; WebView print-to-PDF; drag-drop import on Mac/Windows; real Whisper accuracy.

## Next steps
1. User pushes repo to GitHub, runs "Build installers"; fix CI failures from logs.
2. Mac system-audio capture (ScreenCaptureKit) for parity with the Swift app.
3. Optional: Vulkan GPU build for Windows engines; Intel Mac target; signing.
