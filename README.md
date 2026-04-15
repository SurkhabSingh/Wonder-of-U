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

Phases 1, 2, and 3 are complete with:

- Tauri desktop shell
- tray-first runtime behavior
- global hotkey wiring
- strict recorder state shell
- real Windows system-audio recording
- Whisper runtime/model management, detection, and manual path overrides
- automatic transcript generation beside saved recordings when Whisper is ready
- transcript-based default file renaming after successful transcription

Phase 4 is now in progress with:

- managed translation model downloads into the app asset folder
- user-installed local Python runtime detection or manual runtime path overrides
- app-managed installation of `ctranslate2`, `transformers`, `sentencepiece`, and `protobuf` into the selected Python runtime
- manual translation model directory overrides when you already have a local CTranslate2 model
- selectable translation model footprints in the desktop UI
- target-language selection in the desktop UI
- translation readiness checks for the selected local Python runtime and dependencies
- translation text files saved beside transcripts as `recording_name.translation.<lang>.txt`

AI analysis and Anki land in later phases.

## Repository Notes

- The browser extension prototype in the parent folder is intentionally kept
  separate.
- Runtime assets and downloaded models should eventually live outside the source
  tree and be managed by the app.
