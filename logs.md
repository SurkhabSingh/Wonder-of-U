# Logs

## 2026-04-13

- Simplified the Phase 4 translation runtime design to match the product decision:
  users install Python themselves, and the app installs `ctranslate2`,
  `transformers`, and `sentencepiece` into the selected runtime.
- Removed stale backend code that belonged to the abandoned app-managed Python
  bootstrap path.
- Added the missing translation runtime section to the desktop UI so users can:
  - review the active Python runtime
  - set a manual `python.exe` path
  - browse for a runtime
  - trigger automatic dependency installation
- Updated translation readiness messaging so runtime detection and dependency
  failures describe the current design correctly.
- Validation run:
  - `cargo check --manifest-path src-tauri/Cargo.toml`
  - `cargo test --manifest-path src-tauri/Cargo.toml`
  - `npm run check`
  - `npm run build`

## 2026-04-14

- Fixed the Phase 4 translation runtime dependency list by adding `protobuf`.
- Updated the runtime probe so translation readiness now validates
  `google.protobuf` in addition to `ctranslate2`, `transformers`, and
  `sentencepiece`.
- Fixed the runtime probe script so missing dotted modules such as
  `google.protobuf` are reported as missing dependencies instead of crashing the
  probe and surfacing as a false "Python 3 was not available" error.
- Fixed the translation bridge for managed NLLB models that ship `tokenizer.json`
  without a SentencePiece vocabulary file. The bridge now prefers
  `NllbTokenizerFast` when `tokenizer.json` is present and falls back to the
  slow tokenizer only when needed.
- Suppressed irrelevant Transformers backend warnings inside the translation
  bridge so real tokenizer/model errors are easier to read.
- Updated user-facing copy to reflect the complete translation dependency set.
