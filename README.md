# Wonder of U Desktop

Windows-first offline desktop application workspace.

## Vision

Build a background tray app that can:

- record system audio
- transcribe locally
- translate locally with CTranslate2
- optionally create Anki cards
- remain lightweight while idle

## Current Status

Phases 1 and 2 are complete, and Phase 3 has started with:

- Tauri desktop shell
- tray-first runtime behavior
- global hotkey wiring
- strict recorder state shell
- real Windows system-audio recording
- Whisper install detection and manual path overrides
- automatic transcript generation beside saved recordings when Whisper is ready

Managed asset downloads, offline translation, and Anki land in later phases.

## Repository Notes

- The browser extension prototype in the parent folder is intentionally kept
  separate.
- Runtime assets and downloaded models should eventually live outside the source
  tree and be managed by the app.
