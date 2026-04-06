# Roadmap

## Phase 1: Foundation

Status: complete

- choose the desktop shell
- set up tray/background app behavior
- add global hotkeys
- implement the core recorder state machine
- add structured logging and local settings storage

## Phase 2: Recording

Status: complete

- capture system audio on Windows
- save audio locally first
- support user-chosen output folder
- add naming flow and recent recordings list

## Phase 3: Offline Transcription

Status: in progress

- integrate local Whisper runtime
- support managed asset downloads
- verify model/runtime health before first use
- save transcript as text beside the recording

## Phase 4: Offline Translation

- integrate local CTranslate2 translation
- support target-language selection
- save translation as a separate text file with the same base name

## Phase 5: Anki

- add optional Anki card creation
- support offline queueing and manual push
- keep transcript and translation on the card back

## Phase 6: Production Hardening

- add installer/update flow
- add asset versioning and cleanup
- add crash reporting and diagnostics
- run integration and release testing
