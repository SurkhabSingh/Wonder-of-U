# Wonder of U Desktop

Windows-first offline desktop application workspace.

## Vision

Build a background tray app that can:

- record system audio
- transcribe locally
- translate locally with CTranslate2
- optionally create Anki cards
- remain lightweight while idle

## Phase 1

The repository currently focuses on the foundation layer:

- Tauri desktop shell
- tray-first runtime behavior
- global hotkey wiring
- strict recorder state shell
- settings model we can evolve in later phases

Audio capture, transcription, translation, and Anki land in later phases once
the shell is stable and tested.

## Repository Notes

- The browser extension prototype in the parent folder is intentionally kept
  separate.
- Runtime assets and downloaded models should eventually live outside the source
  tree and be managed by the app.
