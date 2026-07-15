use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Runtime};

use crate::app_types::{
    RecentRecording, RecordingSegment, RecordingTextDocument, RecordingTexts,
};

use super::find_recent_recording;

/// Upper bound on how much of a text sidecar we load into a document. Transcripts
/// are tiny in practice; this is a guard against a pathologically large file, not
/// a real limit anyone should hit.
const MAX_TEXT_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Read every transcript and translation text tied to a recording.
///
/// The frontend passes a `file_path`, but it is only ever used to look the
/// recording up in history (same trust model as the Anki push path). Every file
/// actually read comes from the persisted record, and each read is confined to
/// the folder of that record's own audio file. Note the sandbox root is itself
/// derived from the persisted audio path, so the precise guarantee is "cannot
/// read outside the recording's own directory" — it assumes an untampered audio
/// path rather than anchoring to a fixed trusted root. Writing a tampered path
/// into the state file already requires the same local FS access needed to read
/// the target directly, so this is not an escalation on a single-user desktop.
pub(crate) fn read_recording_texts_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
) -> Result<RecordingTexts, String> {
    let recording = find_recent_recording(app, file_path)?;
    // Sandbox to the recording's own folder rather than the global output
    // directory: this stays correct even if the user repoints output later, and
    // every sidecar lives beside the audio.
    let sandbox_root = Path::new(&recording.file_path)
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "The recording path does not have a parent folder.".to_string())?;
    Ok(collect_recording_texts(&sandbox_root, &recording))
}

/// Pure, Tauri-free core: given the sandbox folder and a recording record,
/// resolve and read its transcript and translation text. Never fails as a whole
/// — an unreadable or out-of-sandbox file becomes a `missing` document so the
/// other panes still render.
pub(crate) fn collect_recording_texts(
    sandbox_root: &Path,
    recording: &RecentRecording,
) -> RecordingTexts {
    let mut transcripts = Vec::new();

    for transcript in &recording.transcripts {
        transcripts.push(read_recording_text_document(
            sandbox_root,
            &transcript.file_path,
            transcript.language.clone(),
            transcript.detected_language.clone(),
            transcript.segments_path.as_deref(),
        ));
    }

    // Older recordings carry a single `transcript_path` with no `transcripts`
    // entries. Include it, but skip it when it is the same file already listed
    // above so the primary transcript is not shown twice.
    if let Some(transcript_path) = recording.transcript_path.as_deref() {
        let already_listed = recording
            .transcripts
            .iter()
            .any(|transcript| same_file(&transcript.file_path, transcript_path));
        if !already_listed {
            transcripts.push(read_recording_text_document(
                sandbox_root,
                transcript_path,
                recording
                    .transcript_language
                    .clone()
                    .unwrap_or_else(|| "auto".to_string()),
                recording.transcript_language.clone(),
                None,
            ));
        }
    }

    let mut translations = Vec::new();
    if let Some(translation_path) = recording.translation_path.as_deref() {
        let language = parse_translation_language(Path::new(translation_path));
        translations.push(read_recording_text_document(
            sandbox_root,
            translation_path,
            language,
            None,
            None,
        ));
    }

    RecordingTexts {
        file_path: recording.file_path.clone(),
        transcripts,
        translations,
    }
}

/// Build a single text document, reading the file inside the sandbox. A file
/// that is missing, unreadable, or outside the sandbox yields `missing: true`
/// with empty text rather than an error.
fn read_recording_text_document(
    sandbox_root: &Path,
    file_path: &str,
    language: String,
    detected_language: Option<String>,
    segments_path: Option<&str>,
) -> RecordingTextDocument {
    let text = read_text_within_sandbox(sandbox_root, Path::new(file_path));
    let segments = segments_path
        .map(|path| read_segments_within_sandbox(sandbox_root, Path::new(path)))
        .unwrap_or_default();
    RecordingTextDocument {
        language,
        detected_language,
        file_path: file_path.to_string(),
        missing: text.is_none(),
        text: text.unwrap_or_default(),
        segments,
    }
}

/// Read and deserialize the `{stem}.{lang}.segments.json` sidecar through the same
/// sandbox guard used for transcripts (the file lives beside the audio, so it
/// passes). A missing, out-of-sandbox, or unparseable sidecar degrades to an empty
/// Vec so the read never fails over absent segments.
fn read_segments_within_sandbox(sandbox_root: &Path, candidate: &Path) -> Vec<RecordingSegment> {
    read_text_within_sandbox(sandbox_root, candidate)
        .and_then(|raw| serde_json::from_str::<Vec<RecordingSegment>>(&raw).ok())
        .unwrap_or_default()
}

/// Read a file only if it resolves inside `sandbox_root`. Returns `None` (never
/// reading the bytes) for anything that escapes the sandbox, does not exist, or
/// cannot be opened.
fn read_text_within_sandbox(sandbox_root: &Path, candidate: &Path) -> Option<String> {
    let resolved = resolve_within(sandbox_root, candidate)?;

    // Guard against loading a huge file: cap oversized ones, otherwise read the
    // whole (tiny) transcript in one shot.
    let metadata = fs::metadata(&resolved).ok()?;
    let bytes = if metadata.len() > MAX_TEXT_FILE_BYTES {
        let file = fs::File::open(&resolved).ok()?;
        let mut buffer = Vec::new();
        file.take(MAX_TEXT_FILE_BYTES)
            .read_to_end(&mut buffer)
            .ok()?;
        buffer
    } else {
        fs::read(&resolved).ok()?
    };

    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Canonicalize both the sandbox root and the candidate — resolving `..`,
/// symlinks, and the Windows `\\?\` verbatim prefix on *both* sides — and
/// confirm the candidate stays inside the root. `None` means the candidate does
/// not exist or escapes the sandbox.
fn resolve_within(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let canonical_root = root.canonicalize().ok()?;
    let canonical_candidate = candidate.canonicalize().ok()?;
    canonical_candidate
        .starts_with(&canonical_root)
        .then_some(canonical_candidate)
}

/// Pull the `{lang}` out of a `{stem}.translation.{lang}.txt` sidecar name,
/// defaulting to `"en"` when the name does not carry a language tag.
fn parse_translation_language(path: &Path) -> String {
    let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
    let without_extension = file_name.strip_suffix(".txt").unwrap_or(file_name);
    match without_extension.rsplit_once(".translation.") {
        Some((_, language)) if !language.is_empty() => language.to_string(),
        _ => "en".to_string(),
    }
}

/// Treat two stored paths as the same file when they canonicalize to the same
/// location (so a redundant `.`/`..` or symlink does not defeat de-duplication),
/// falling back to a raw string match when either path cannot be canonicalized.
fn same_file(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    match (
        Path::new(left).canonicalize(),
        Path::new(right).canonicalize(),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_types::RecordingTranscript;

    fn recording_with(file_path: &str) -> RecentRecording {
        RecentRecording {
            file_name: "recording.wav".into(),
            file_path: file_path.into(),
            transcript_path: None,
            transcript_language: None,
            transcripts: Vec::new(),
            translation_path: None,
            anki_note_id: None,
            anki_deck_name: None,
            anki_note_type: None,
            anki_pushes: Vec::new(),
            furigana_applied: false,
            audio_deleted: false,
            duration_ms: 0,
            bytes_written: 0,
            created_at_ms: 0,
            source: None,
            source_url: None,
            title: None,
        }
    }

    #[test]
    fn reads_transcript_text() {
        let dir = tempfile::tempdir().unwrap();
        let audio_path = dir.path().join("clip.wav");
        fs::write(&audio_path, b"audio").unwrap();
        let transcript_path = dir.path().join("clip.transcript.txt");
        fs::write(&transcript_path, "hello world").unwrap();

        let mut recording = recording_with(&audio_path.display().to_string());
        recording.transcripts.push(RecordingTranscript {
            language: "en".into(),
            file_path: transcript_path.display().to_string(),
            detected_language: Some("en".into()),
            segments_path: None,
        });

        let texts = collect_recording_texts(dir.path(), &recording);

        assert_eq!(texts.file_path, audio_path.display().to_string());
        assert_eq!(texts.transcripts.len(), 1);
        let document = &texts.transcripts[0];
        assert_eq!(document.text, "hello world");
        assert!(!document.missing);
        assert_eq!(document.language, "en");
        assert_eq!(document.detected_language.as_deref(), Some("en"));
    }

    #[test]
    fn returns_one_document_per_language() {
        let dir = tempfile::tempdir().unwrap();
        let audio_path = dir.path().join("clip.wav");
        fs::write(&audio_path, b"audio").unwrap();
        let english_path = dir.path().join("clip.en.transcript.txt");
        fs::write(&english_path, "hello").unwrap();
        let japanese_path = dir.path().join("clip.ja.transcript.txt");
        fs::write(&japanese_path, "こんにちは").unwrap();

        let mut recording = recording_with(&audio_path.display().to_string());
        recording.transcripts.push(RecordingTranscript {
            language: "en".into(),
            file_path: english_path.display().to_string(),
            detected_language: Some("en".into()),
            segments_path: None,
        });
        recording.transcripts.push(RecordingTranscript {
            language: "ja".into(),
            file_path: japanese_path.display().to_string(),
            detected_language: Some("ja".into()),
            segments_path: None,
        });

        let texts = collect_recording_texts(dir.path(), &recording);

        assert_eq!(texts.transcripts.len(), 2);
        assert_eq!(texts.transcripts[0].language, "en");
        assert_eq!(texts.transcripts[0].text, "hello");
        assert_eq!(texts.transcripts[1].language, "ja");
        assert_eq!(texts.transcripts[1].text, "こんにちは");
    }

    #[test]
    fn parses_translation_language_from_filename() {
        assert_eq!(
            parse_translation_language(Path::new("clip.translation.fr.txt")),
            "fr"
        );
        assert_eq!(
            parse_translation_language(Path::new("a.b.c_100.translation.zh-hans.txt")),
            "zh-hans"
        );
        // No language tag in the name falls back to English.
        assert_eq!(
            parse_translation_language(Path::new("clip.transcript.txt")),
            "en"
        );

        let dir = tempfile::tempdir().unwrap();
        let audio_path = dir.path().join("clip.wav");
        fs::write(&audio_path, b"audio").unwrap();
        let translation_path = dir.path().join("clip.translation.fr.txt");
        fs::write(&translation_path, "bonjour").unwrap();

        let mut recording = recording_with(&audio_path.display().to_string());
        recording.translation_path = Some(translation_path.display().to_string());

        let texts = collect_recording_texts(dir.path(), &recording);
        assert_eq!(texts.translations.len(), 1);
        assert_eq!(texts.translations[0].language, "fr");
        assert_eq!(texts.translations[0].text, "bonjour");
        assert!(!texts.translations[0].missing);
    }

    #[test]
    fn missing_file_is_flagged_not_errored() {
        let dir = tempfile::tempdir().unwrap();
        let audio_path = dir.path().join("clip.wav");
        fs::write(&audio_path, b"audio").unwrap();
        // Point at a transcript that was never written to disk.
        let transcript_path = dir.path().join("clip.transcript.txt");

        let mut recording = recording_with(&audio_path.display().to_string());
        recording.transcripts.push(RecordingTranscript {
            language: "en".into(),
            file_path: transcript_path.display().to_string(),
            detected_language: None,
            segments_path: None,
        });

        let texts = collect_recording_texts(dir.path(), &recording);

        assert_eq!(texts.transcripts.len(), 1);
        assert!(texts.transcripts[0].missing);
        assert_eq!(texts.transcripts[0].text, "");
    }

    #[test]
    fn rejects_path_outside_sandbox() {
        let sandbox = tempfile::tempdir().unwrap();
        let audio_path = sandbox.path().join("clip.wav");
        fs::write(&audio_path, b"audio").unwrap();

        // A real, readable file that simply lives outside the recording folder.
        let outside = tempfile::tempdir().unwrap();
        let secret_path = outside.path().join("secret.transcript.txt");
        fs::write(&secret_path, "top secret").unwrap();

        let mut recording = recording_with(&audio_path.display().to_string());
        recording.transcripts.push(RecordingTranscript {
            language: "en".into(),
            file_path: secret_path.display().to_string(),
            detected_language: None,
            segments_path: None,
        });

        let texts = collect_recording_texts(sandbox.path(), &recording);

        // The file exists and is readable, yet it must be rejected without its
        // contents ever being surfaced.
        assert_eq!(texts.transcripts.len(), 1);
        assert!(texts.transcripts[0].missing);
        assert_eq!(texts.transcripts[0].text, "");
    }

    #[test]
    fn decodes_invalid_utf8_lossily() {
        let dir = tempfile::tempdir().unwrap();
        let audio_path = dir.path().join("clip.wav");
        fs::write(&audio_path, b"audio").unwrap();
        let transcript_path = dir.path().join("clip.transcript.txt");
        // Valid ASCII followed by a lone continuation byte (invalid UTF-8).
        fs::write(&transcript_path, b"ok\xffend").unwrap();

        let mut recording = recording_with(&audio_path.display().to_string());
        recording.transcripts.push(RecordingTranscript {
            language: "en".into(),
            file_path: transcript_path.display().to_string(),
            detected_language: None,
            segments_path: None,
        });

        let texts = collect_recording_texts(dir.path(), &recording);

        assert_eq!(texts.transcripts.len(), 1);
        assert!(!texts.transcripts[0].missing);
        assert_eq!(texts.transcripts[0].text, "ok\u{FFFD}end");
    }

    #[test]
    fn de_duplicates_transcript_path_against_transcripts() {
        let dir = tempfile::tempdir().unwrap();
        let audio_path = dir.path().join("clip.wav");
        fs::write(&audio_path, b"audio").unwrap();
        let transcript_path = dir.path().join("clip.transcript.txt");
        fs::write(&transcript_path, "hello").unwrap();

        let mut recording = recording_with(&audio_path.display().to_string());
        recording.transcripts.push(RecordingTranscript {
            language: "en".into(),
            file_path: transcript_path.display().to_string(),
            detected_language: Some("en".into()),
            segments_path: None,
        });
        // The same file reached by a different-but-equivalent path (redundant
        // `.` component) must still be recognized as a duplicate.
        recording.transcript_path = Some(
            dir.path()
                .join(".")
                .join("clip.transcript.txt")
                .display()
                .to_string(),
        );
        recording.transcript_language = Some("en".into());

        let texts = collect_recording_texts(dir.path(), &recording);

        assert_eq!(
            texts.transcripts.len(),
            1,
            "the primary transcript must not be listed twice"
        );
        assert_eq!(texts.transcripts[0].text, "hello");
    }

    #[test]
    fn reads_segments_sidecar_beside_audio() {
        let dir = tempfile::tempdir().unwrap();
        let audio_path = dir.path().join("clip.wav");
        fs::write(&audio_path, b"audio").unwrap();
        let transcript_path = dir.path().join("clip.transcript.txt");
        fs::write(&transcript_path, "hello world").unwrap();
        let segments_path = dir.path().join("clip.en.segments.json");
        fs::write(
            &segments_path,
            r#"[{"text":"hello","startMs":0,"endMs":1500},{"text":"world","startMs":1500,"endMs":2960}]"#,
        )
        .unwrap();

        let mut recording = recording_with(&audio_path.display().to_string());
        recording.transcripts.push(RecordingTranscript {
            language: "en".into(),
            file_path: transcript_path.display().to_string(),
            detected_language: Some("en".into()),
            segments_path: Some(segments_path.display().to_string()),
        });

        let texts = collect_recording_texts(dir.path(), &recording);

        assert_eq!(texts.transcripts.len(), 1);
        let document = &texts.transcripts[0];
        assert_eq!(document.segments.len(), 2);
        assert_eq!(document.segments[0].text, "hello");
        assert_eq!(document.segments[0].start_ms, 0);
        assert_eq!(document.segments[0].end_ms, 1500);
        assert_eq!(document.segments[1].text, "world");
        assert_eq!(document.segments[1].end_ms, 2960);
    }

    #[test]
    fn missing_or_unparseable_segments_degrade_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let audio_path = dir.path().join("clip.wav");
        fs::write(&audio_path, b"audio").unwrap();
        let transcript_path = dir.path().join("clip.transcript.txt");
        fs::write(&transcript_path, "hello").unwrap();
        // A sidecar path that was never written.
        let segments_path = dir.path().join("clip.en.segments.json");

        let mut recording = recording_with(&audio_path.display().to_string());
        recording.transcripts.push(RecordingTranscript {
            language: "en".into(),
            file_path: transcript_path.display().to_string(),
            detected_language: Some("en".into()),
            segments_path: Some(segments_path.display().to_string()),
        });

        let texts = collect_recording_texts(dir.path(), &recording);

        assert_eq!(texts.transcripts.len(), 1);
        assert!(!texts.transcripts[0].missing);
        assert!(
            texts.transcripts[0].segments.is_empty(),
            "a missing segments sidecar must not error the read"
        );
    }
}
