# Todo

## Phase 4 Remaining

- Re-run translation dependency installation on a local Python runtime so the
  newly added `protobuf` package is installed before the next end-to-end test.
- Verify that missing translation packages now surface as `dependenciesMissing`
  instead of the false "Python 3 was not available" runtime error.
- Verify the full translation execution path against a real local Python runtime
  with `ctranslate2`, `transformers`, and `sentencepiece` installed by the app.
- Verify translation against the managed `distilled-1.3b-int8` model specifically,
  since it relies on `tokenizer.json` and the fast tokenizer path.
- Confirm end-to-end output creation:
  - transcript exists
  - translation model exists
  - translated `*.translation.<lang>.txt` file is written beside the transcript
- Add update-check support for managed translation models if needed after the
  base flow is stable.
- Decide whether translation runtime health should expose package versions in
  the UI or remain readiness-only.

## Known Causes To Remember

- The abandoned app-managed Python bootstrap path created unnecessary runtime
  complexity and stale detection logic. Do not reintroduce that design unless
  the product decision changes.
- Translation runtime setup should target a user-installed Python executable and
  install pinned dependencies from the app-owned requirements file.
