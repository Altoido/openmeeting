# OpenMeeting Notes

Free, private, **offline** meeting recorder, transcriber and AI note-taker for **macOS and Windows**.
Record → Transcribe → TL;DR + Summary → Ask AI → Export. No account, subscription, telemetry or cloud AI.

## Download (for your team)

Get the latest installers from this repository's **Releases** page:

- **Mac (Apple Silicon):** download the `.dmg`, open it, drag **OpenMeeting Notes** into Applications.
  First launch: right-click the app → **Open** → **Open**. On macOS 15 or later, if it's blocked: System Settings → Privacy & Security → **Open Anyway**.
- **Windows 10/11 (64-bit):** download `…_x64-setup.exe` and run it. If "Windows protected your PC" appears, click **More info → Run anyway**.

These warnings appear because the app isn't signed with paid Apple/Microsoft certificates. Signing can be added later (see below).

On first launch the setup screen recommends AI models for that computer and downloads them once (about 1.6–3 GB). After that everything works offline.

## Features

- Recording: microphone (Mac & Windows); computer audio and mic + computer audio (Windows, via WASAPI loopback). Pause/resume, live levels, typed notes, highlights, crash-safe 1-minute chunks with automatic recovery.
- Import: MP3, WAV, M4A/AAC, FLAC, OGG, MP4, MOV (decoded on-device).
- Transcription: whisper.cpp, ~99 languages, 10-minute checkpointed passes, silence detection, timestamps, click-to-play, editing (original kept), speaker labels (automatic Me/Remote for two-source recordings; manual otherwise).
- AI (llama.cpp, Qwen3 models): one-page **TL;DR**, 10 summary templates + custom templates, hierarchical summarization for long meetings, action items, AI titles, mind maps, Ask AI about one meeting or the whole library.
- Updates: a "new version available" prompt linked to this repository's Releases.
- Library: search across titles, dates, transcripts, summaries and notes (SQLite FTS5), tags, collections, favorites, archive.
- Export: PDF (print → Save as PDF), Word, Markdown, text, SRT, VTT, original audio.

## How releases are built

`.github/workflows/release.yml` builds both installers on GitHub's Mac and Windows machines:
1. compiles the local AI engines (`scripts/build-engines.sh`, pinned whisper.cpp v1.9.5 and llama.cpp b12441),
2. runs the core tests,
3. builds the app with Tauri and attaches the `.dmg` and `-setup.exe` to a GitHub Release.

Trigger it from the **Actions** tab → *Build installers* → **Run workflow**. The release is named after the version in `package.json` (e.g. **1.1.0**); running it again for the same version replaces that release's installers.

## Updates

The app checks this repository's latest Release about twice a day and shows a "new version available" bar with a Download button (Settings → About can turn this off or check now). Nothing is installed automatically; installing a new version over an old one keeps all meetings and models.

To publish a new version: bump `version` in `package.json` (the only place it's set), commit, push, then Run workflow.

## Develop locally

Requires Node 20+, Rust, CMake, and (Mac) Xcode or (Windows) Visual Studio Build Tools.
```
npm install
./scripts/build-engines.sh          # once
npm run tauri dev                   # run the app
cd src-tauri && cargo test --lib    # core tests
OMN_TEST_LLM=/path/model.gguf OMN_ENGINES_DIR=engines-build/lb/bin cargo test --lib live_pipeline -- --ignored --nocapture
```

## Data locations

- Mac: `~/Library/Application Support/OpenMeeting Notes/` · Windows: `%APPDATA%\OpenMeeting Notes\`
- `Library/` holds the database and recordings; `Models/` holds AI models. Delete a meeting from its ⋯ menu.

## Optional: remove the security warnings

- Mac: Apple Developer Program ($99/yr) → Developer ID certificate → add `APPLE_CERTIFICATE`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` secrets (tauri-action signs and notarizes).
- Windows: Microsoft Trusted Signing (~$10/mo) or a code-signing certificate, configured under `bundle.windows` in `src-tauri/tauri.conf.json`.

## Known limitations (v1.0)

- Mac: computer-audio capture (Zoom/Meet/Teams on the Mac itself) isn't in this cross-platform version yet — record with the microphone. Planned: ScreenCaptureKit support.
- Windows and Intel Macs run the AI on the processor, so summaries take longer than on Apple Silicon (a 1-hour meeting may take several minutes).
- Only Apple Silicon Macs are built by default (add an `x86_64-apple-darwin` row to the workflow for Intel Macs).

## License
MIT. Third-party: whisper.cpp & llama.cpp (MIT), Tauri (MIT/Apache-2.0), Symphonia (MPL-2.0), cpal (Apache-2.0). Models are downloaded separately: Whisper (MIT), Silero VAD (MIT), Qwen3 (Apache 2.0).
