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

Status: complete

- integrate local Whisper runtime
- support managed asset downloads
- verify model/runtime health before first use
- save transcript as text beside the recording
- rename saved recording outputs from the transcript by default

## Phase 4: Offline Translation

Status: in progress

- integrate local CTranslate2 translation
- rely on a user-installed Python runtime instead of an app-managed Python bootstrap
- install `ctranslate2`, `transformers`, and `sentencepiece` automatically into the selected local Python runtime
- support runtime detection plus manual `python.exe` overrides
- keep translation model downloads managed by the app
- support target-language selection
- save translation as a separate text file with the same base name

## Phase 5: AI Analysis Layer

Status: planned

- add an optional provider-agnostic AI post-processing layer after transcription and translation
- support at most 4 providers in v1:
  - Gemini
  - Groq
  - OpenAI-compatible generic adapter
  - Ollama
- keep free-first provider support as the priority
- build reusable prompt profiles instead of raw one-off prompts
- support provider/model/profile-specific approval for first-run output review
- require preview and user approval for every new prompt profile
- allow automatic reuse after approval until the profile meaningfully changes
- use structured outputs for language-learning tasks wherever possible
- save AI outputs locally in both machine-friendly and human-friendly forms
- prepare AI outputs to become stable inputs for the later Anki phase

## Phase 6: Anki

Status: planned

- add optional Anki card creation
- support offline queueing and manual push
- keep transcript, translation, and later AI-generated study content available for note creation

## Phase 7: Production Hardening

Status: planned

- add installer/update flow
- add asset versioning and cleanup
- add crash reporting and diagnostics
- run integration and release testing
