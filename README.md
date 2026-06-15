# Wonder of U

Wonder of U is a local-first desktop application for capturing system audio,
transcribing speech with Whisper, and preparing language-learning content for
Anki.

## Current Capabilities

- Tauri desktop application with tray-first behavior.
- Global shortcuts for starting and stopping system-audio recording.
- Windows system-output capture saved as local WAV files.
- Local Whisper transcription.
- Managed Whisper runtime and model downloads.
- Automatic discovery of downloaded Whisper assets after restart.
- Manual `whisper-cli` and GGML model path overrides.
- Download progress, pause, resume, cancellation, and update checks.
- Configurable output and asset directories.
- Recent recording and transcript history.

## Product Features

The desktop application is being extended to support:

- Windows, macOS, and Linux system-audio capture.
- Audio-only and audio-plus-visual-context shortcuts.
- Low-frame-rate screenshot capture encoded into compact H.264 MP4 files.
- Browser-extension-assisted translation.
- Japanese furigana generation with editable readings.
- Existing Anki deck, note type, and field mapping.
- Persistent card review and retry queue.
- Light and dark themes.

Audio, transcripts, and generated media remain local unless the user explicitly
uses a configured translation service or sends a reviewed note to Anki.

## Development

```powershell
npm install
npm run check
npm run build
cargo test --manifest-path .\src-tauri\Cargo.toml
```
