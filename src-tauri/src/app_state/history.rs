use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::app_types::{
    transcript_language_key, PersistedData, RecentRecording, RecordingAnkiPush, RecordingTranscript,
};

pub(crate) fn reconcile_recording_history(state: &mut PersistedData) {
    let output_directory = PathBuf::from(&state.settings.output_directory);
    let Ok(entries) = fs::read_dir(&output_directory) else {
        return;
    };

    let mut known_paths = state
        .recent_recordings
        .iter()
        .map(|recording| normalized_path_key(Path::new(&recording.file_path)))
        .collect::<HashSet<_>>();

    for entry in entries.flatten() {
        let audio_path = entry.path();
        if !entry
            .file_type()
            .map(|kind| kind.is_file())
            .unwrap_or(false)
            || !is_supported_audio_path(&audio_path)
        {
            continue;
        }

        let path_key = normalized_path_key(&audio_path);
        if !known_paths.insert(path_key) {
            continue;
        }

        if let Some(recording) = recording_from_audio_path(&audio_path) {
            state.recent_recordings.push(recording);
        }
    }

    state
        .recent_recordings
        .sort_by(|left, right| right.created_at_ms.cmp(&left.created_at_ms));
}

fn is_supported_audio_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            extension.eq_ignore_ascii_case("wav") || extension.eq_ignore_ascii_case("mp3")
        })
        .unwrap_or(false)
}

fn normalized_path_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn recording_from_audio_path(audio_path: &Path) -> Option<RecentRecording> {
    let metadata = fs::metadata(audio_path).ok()?;
    let parent = audio_path.parent()?;
    let stem = audio_path.file_stem()?.to_str()?;
    let transcript_path = parent.join(format!("{stem}.transcript.txt"));
    let transcript_path = transcript_path
        .is_file()
        .then(|| transcript_path.display().to_string());
    let translation_path = find_translation_path(parent, stem);
    let created_at_ms =
        earliest_file_timestamp_ms(audio_path, transcript_path.as_deref().map(Path::new));

    Some(RecentRecording {
        file_name: audio_path.file_name()?.to_str()?.to_string(),
        file_path: audio_path.display().to_string(),
        transcript_path,
        transcript_language: None,
        transcripts: Vec::new(),
        translation_path,
        anki_note_id: None,
        anki_deck_name: None,
        anki_note_type: None,
        anki_pushes: Vec::new(),
        furigana_applied: false,
        audio_deleted: false,
        duration_ms: wav_duration_ms(audio_path).unwrap_or(0),
        bytes_written: metadata.len(),
        created_at_ms,
    })
}

fn find_translation_path(directory: &Path, stem: &str) -> Option<String> {
    let prefix = format!("{stem}.translation.");
    fs::read_dir(directory)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.starts_with(&prefix) && name.ends_with(".txt"))
                    .unwrap_or(false)
        })
        .map(|path| path.display().to_string())
}

fn earliest_file_timestamp_ms(audio_path: &Path, transcript_path: Option<&Path>) -> u64 {
    [Some(audio_path), transcript_path]
        .into_iter()
        .flatten()
        .filter_map(file_timestamp_ms)
        .min()
        .unwrap_or_else(current_time_ms)
}

fn file_timestamp_ms(path: &Path) -> Option<u64> {
    let metadata = fs::metadata(path).ok()?;
    metadata
        .created()
        .or_else(|_| metadata.modified())
        .ok()
        .and_then(system_time_ms)
}

fn system_time_ms(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

fn current_time_ms() -> u64 {
    system_time_ms(SystemTime::now()).unwrap_or(0)
}

fn wav_duration_ms(path: &Path) -> Option<u64> {
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("wav"))
        .unwrap_or(false)
    {
        return None;
    }

    let reader = hound::WavReader::open(path).ok()?;
    let sample_rate = reader.spec().sample_rate;
    (sample_rate > 0).then(|| reader.duration() as u64 * 1000 / sample_rate as u64)
}

pub(super) fn normalize_recent_recording_languages(recordings: &mut [RecentRecording]) {
    for recording in recordings {
        for transcript in &mut recording.transcripts {
            transcript.language = transcript_language_key(&transcript.language);
        }

        if recording.transcript_language.is_none() {
            if let Some(transcript_path) = recording.transcript_path.as_deref() {
                recording.transcript_language =
                    derive_transcript_language_from_path(Path::new(transcript_path), "auto");
            }
        }

        if recording.transcripts.is_empty() {
            if let Some(transcript_path) = recording.transcript_path.clone() {
                recording.transcripts.push(RecordingTranscript {
                    language: transcript_language_key(
                        recording.transcript_language.as_deref().unwrap_or("auto"),
                    ),
                    file_path: transcript_path,
                    detected_language: recording.transcript_language.clone(),
                    segments_path: None,
                });
            }
        }

        for push in &mut recording.anki_pushes {
            push.language = transcript_language_key(&push.language);
        }

        if recording.anki_pushes.is_empty() {
            if let (Some(note_id), Some(deck_name), Some(note_type)) = (
                recording.anki_note_id,
                recording.anki_deck_name.clone(),
                recording.anki_note_type.clone(),
            ) {
                let language = recording
                    .transcript_path
                    .as_deref()
                    .and_then(|transcript_path| {
                        recording
                            .transcripts
                            .iter()
                            .find(|transcript| transcript.file_path == transcript_path)
                    })
                    .map(|transcript| transcript.language.as_str())
                    .unwrap_or_else(|| recording.transcript_language.as_deref().unwrap_or("auto"));
                recording.anki_pushes.push(RecordingAnkiPush {
                    language: transcript_language_key(language),
                    deck_name,
                    note_type,
                    note_id,
                    furigana_applied: recording.furigana_applied,
                });
            }
        }
    }
}

pub(crate) fn derive_transcript_language_from_path(
    transcript_path: &Path,
    requested_language: &str,
) -> Option<String> {
    if let Some(language) = normalize_transcript_language_code(requested_language) {
        return Some(language);
    }

    let transcript = fs::read_to_string(transcript_path).ok()?;
    if transcript_looks_japanese(&transcript) {
        Some("ja".into())
    } else {
        None
    }
}

fn normalize_transcript_language_code(language: &str) -> Option<String> {
    let normalized = language.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "" | "auto" => None,
        "ja" | "japanese" => Some("ja".into()),
        _ => Some(normalized),
    }
}

pub(crate) fn transcript_looks_japanese(transcript: &str) -> bool {
    transcript.chars().any(is_japanese_kana)
}

fn is_japanese_kana(character: char) -> bool {
    matches!(
        character as u32,
        0x3040..=0x30ff | 0x31f0..=0x31ff | 0xff66..=0xff9f
    )
}

pub(crate) fn is_japanese_transcript_language(language: Option<&str>) -> bool {
    language
        .map(normalize_transcript_language_code)
        .is_some_and(|normalized| normalized.as_deref() == Some("ja"))
}
