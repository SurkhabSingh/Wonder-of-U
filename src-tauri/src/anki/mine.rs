use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use tauri::{AppHandle, Manager, Runtime};

use super::{
    client::{anki_connect_request, anki_offline_message},
    fields::{
        anki_media_file_name, html_escape, prepend_anki_field_value, preserve_anki_sound_tags,
        user_friendly_anki_error,
    },
    furigana::{recording_transcript_supports_furigana, request_furigana_html},
};
use crate::{
    app_runtime::{build_app_bootstrap, update_shell_snapshot},
    app_types::{
        AppSettings, RecentRecording, RecordingActionItem, RecordingBatchResult,
        SharedPersistedState,
    },
    recording_library::{find_recent_recording, playback_path},
    runtime_assets::detect_local_ffmpeg,
};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Padding added to each side of the requested segment so the clip does not clip
/// the first or last syllable. Clamped to the start of the file on the low side.
const SEGMENT_PADDING_MS: u64 = 250;

fn hide_command_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

fn unique_path_with_suffix(directory: &Path, file_stem: &str, suffix: &str) -> PathBuf {
    let sanitized_stem = if file_stem.is_empty() {
        "recording".to_string()
    } else {
        file_stem.to_string()
    };

    let mut attempt = 0usize;
    loop {
        let candidate = if attempt == 0 {
            directory.join(format!("{sanitized_stem}{suffix}"))
        } else {
            directory.join(format!("{sanitized_stem}_{attempt}{suffix}"))
        };

        if !candidate.exists() {
            return candidate;
        }

        attempt += 1;
    }
}

/// Formats a millisecond offset as the `S.mmm` seconds string ffmpeg expects for
/// `-ss`/`-to` (e.g. `1500` -> `"1.500"`, `250` -> `"0.250"`).
fn format_ffmpeg_timestamp(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

/// Builds the ffmpeg argument list for slicing `[start_ms, end_ms]` (with padding)
/// out of `input` into `output`. Kept pure so the ordering and timestamp
/// formatting can be unit-tested without spawning ffmpeg. `-ss`/`-to` come before
/// `-i` so ffmpeg seeks by keyframe before decoding.
fn slice_ffmpeg_args(start_ms: u64, end_ms: u64, input: &str, output: &str) -> Vec<String> {
    let start = start_ms.saturating_sub(SEGMENT_PADDING_MS);
    let end = end_ms.saturating_add(SEGMENT_PADDING_MS);
    vec![
        "-y".into(),
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-ss".into(),
        format_ffmpeg_timestamp(start),
        "-to".into(),
        format_ffmpeg_timestamp(end),
        "-i".into(),
        input.to_string(),
        "-map".into(),
        "0:a:0".into(),
        "-vn".into(),
        "-codec:a".into(),
        "libmp3lame".into(),
        "-b:a".into(),
        "128k".into(),
        output.to_string(),
    ]
}

/// Slices the requested sentence out of `audio_path` into a fresh MP3 beside it.
/// FFmpeg is mandatory here: unlike the optional WAV->MP3 compression, a mine has
/// nothing to attach without the clip, so a missing binary is a hard error.
fn slice_segment_clip(
    settings: &AppSettings,
    audio_path: &Path,
    start_ms: u64,
    end_ms: u64,
) -> Result<PathBuf, String> {
    let detection = detect_local_ffmpeg(settings);
    let executable_path = detection
        .executable_path
        .clone()
        .ok_or_else(|| "FFmpeg is required to mine audio; install it in Setup.".to_string())?;

    let parent = audio_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = audio_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("recording");
    let clip_path = unique_path_with_suffix(parent, &format!("{stem}_seg{start_ms}"), ".mp3");

    let mut command = Command::new(&executable_path);
    hide_command_window(&mut command);
    if let Some(ffmpeg_directory) = Path::new(&executable_path).parent() {
        command.current_dir(ffmpeg_directory);
    }
    command.args(slice_ffmpeg_args(
        start_ms,
        end_ms,
        &audio_path.display().to_string(),
        &clip_path.display().to_string(),
    ));

    let output = command.output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "FFmpeg is required to mine audio; install it in Setup.".to_string()
        } else {
            format!("FFmpeg could not slice the audio clip: {error}")
        }
    })?;

    let clip_ready = output.status.success()
        && fs::metadata(&clip_path)
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false);

    if !clip_ready {
        let _ = fs::remove_file(&clip_path);
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "FFmpeg did not produce an audio clip for this sentence.".to_string()
        } else {
            format!("FFmpeg could not slice the audio clip: {stderr}")
        });
    }

    Ok(clip_path)
}

/// Merges furigana into the transcription field when the sentence reads as
/// Japanese, mirroring the push flow's overwrite-with-preserved-sound-tags
/// behavior. Non-fatal: a lookup miss leaves the plain transcription in place.
fn maybe_merge_furigana(
    recording: &RecentRecording,
    settings: &crate::app_types::AnkiSettings,
    text: &str,
    clip_media_file_name: &str,
    fields: &mut serde_json::Map<String, serde_json::Value>,
) {
    if !recording_transcript_supports_furigana(recording, text) {
        return;
    }

    let Ok(furigana_html) = request_furigana_html(text) else {
        return;
    };

    let target_field = settings.fields.transcription.as_str();
    let existing_value = fields.get(target_field).and_then(|value| value.as_str());
    let fallback_sound_tag =
        if !settings.fields.audio.is_empty() && settings.fields.audio == target_field {
            Some(format!("[sound:{clip_media_file_name}]"))
        } else {
            None
        };
    let merged = preserve_anki_sound_tags(existing_value, &furigana_html, fallback_sound_tag.as_deref());
    fields.insert(target_field.to_string(), serde_json::Value::String(merged));
}

/// Runs the whole mine for one sentence and returns the single action item plus
/// the batch status string that item maps to.
fn mine_single_segment<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
    text: &str,
    start_ms: u64,
    end_ms: u64,
    translation: Option<&str>,
) -> (RecordingActionItem, &'static str) {
    let failed = |message: String| {
        (
            RecordingActionItem {
                file_path: file_path.to_string(),
                status: "failed".into(),
                message,
                note_id: None,
            },
            "partial",
        )
    };

    // 1. Settings + field-mapping validation.
    let settings = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = match persisted_state.0.lock() {
            Ok(persisted) => persisted,
            Err(_) => return failed("Could not read the Anki settings.".into()),
        };
        persisted.settings.clone()
    };
    let anki = settings.anki.clone();
    if anki.deck_name.is_empty() {
        return failed("Choose an Anki deck before mining sentences.".into());
    }
    if anki.note_type.is_empty() {
        return failed("Choose an Anki note type before mining sentences.".into());
    }
    if anki.fields.transcription.is_empty() {
        return failed("Map an Anki field for the transcript before mining sentences.".into());
    }

    let trimmed_text = text.trim();
    if trimmed_text.is_empty() {
        return failed("There is no sentence text to mine.".into());
    }

    // 2. Resolve the recording and its audio on disk.
    let recording = match find_recent_recording(app, file_path) {
        Ok(recording) => recording,
        Err(error) => return failed(error),
    };
    let audio_path = match playback_path(&recording) {
        Ok(path) => path,
        Err(error) => return failed(error),
    };

    // 3. Slice the sentence clip (ffmpeg is mandatory).
    let clip_path = match slice_segment_clip(&settings, &audio_path, start_ms, end_ms) {
        Ok(path) => path,
        Err(error) => return failed(error),
    };

    // 4. Anki must be reachable before we store media or add the note.
    if let Err(error) = anki_connect_request("version", serde_json::json!({})) {
        let _ = fs::remove_file(&clip_path);
        return (
            RecordingActionItem {
                file_path: file_path.to_string(),
                status: "failed".into(),
                message: anki_offline_message(&error),
                note_id: None,
            },
            "unavailable",
        );
    }

    let clip_media_file_name = anki_media_file_name(&clip_path);
    let store_result = anki_connect_request(
        "storeMediaFile",
        serde_json::json!({
            "filename": clip_media_file_name,
            "path": clip_path.display().to_string()
        }),
    );
    // 8. Anki copies the clip into its own media folder, so the temp file is done
    // regardless of whether storing (or the later addNote) succeeds.
    let _ = fs::remove_file(&clip_path);
    if let Err(error) = store_result {
        return failed(format!("Anki could not store the audio clip. {error}"));
    }

    // 4 (cont.). Build the note fields from the mapping.
    let mut fields = serde_json::Map::new();
    fields.insert(
        anki.fields.transcription.clone(),
        serde_json::Value::String(html_escape(trimmed_text)),
    );
    prepend_anki_field_value(
        &mut fields,
        &anki.fields.audio,
        format!("[sound:{clip_media_file_name}]"),
    );
    if !anki.fields.source_path.is_empty() {
        fields.insert(
            anki.fields.source_path.clone(),
            serde_json::Value::String(html_escape(&recording.file_path)),
        );
    }
    if !anki.fields.created_at.is_empty() {
        fields.insert(
            anki.fields.created_at.clone(),
            serde_json::Value::String(recording.created_at_ms.to_string()),
        );
    }

    // 5. Translation: reuse the recording's existing translation for this
    // sentence when one is present (the paired line the viewer already shows).
    // Mining never generates a fresh translation — if none exists, the card
    // carries the text alone, mirroring the whole-recording push.
    if !anki.fields.translation.is_empty() {
        if let Some(translation) = translation.map(str::trim).filter(|value| !value.is_empty()) {
            fields.insert(
                anki.fields.translation.clone(),
                serde_json::Value::String(html_escape(translation)),
            );
        }
    }

    // 6. Furigana (non-fatal).
    maybe_merge_furigana(&recording, &anki, trimmed_text, &clip_media_file_name, &mut fields);

    // 7. Create the note with the same dedup guard the push flow uses.
    let note_result = anki_connect_request(
        "addNote",
        serde_json::json!({
            "note": {
                "deckName": anki.deck_name.clone(),
                "modelName": anki.note_type.clone(),
                "fields": fields,
                "options": {
                    "allowDuplicate": false,
                    "duplicateScope": "deck",
                    "duplicateScopeOptions": {
                        "deckName": anki.deck_name.clone(),
                        "checkChildren": false,
                        "checkAllModels": false
                    }
                },
                "tags": ["wonder-of-u"]
            }
        }),
    );

    let note_id = match note_result {
        Ok(value) => match value.as_i64() {
            Some(note_id) => note_id,
            None => return failed("AnkiConnect did not return a note id.".into()),
        },
        Err(error) => {
            if error.to_lowercase().contains("duplicate") {
                return (
                    RecordingActionItem {
                        file_path: file_path.to_string(),
                        status: "skipped".into(),
                        message: "This sentence is already mined.".into(),
                        note_id: None,
                    },
                    "completed",
                );
            }
            return failed(user_friendly_anki_error(&error, &anki));
        }
    };

    (
        RecordingActionItem {
            file_path: file_path.to_string(),
            status: "success".into(),
            message: format!("Mined sentence into Anki note {note_id}."),
            note_id: Some(note_id),
        },
        "completed",
    )
}

pub(crate) fn mine_segment_to_anki_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_path: String,
    text: String,
    start_ms: u64,
    end_ms: u64,
    translation: Option<String>,
) -> Result<RecordingBatchResult, String> {
    let (item, batch_status) = mine_single_segment(
        app,
        &file_path,
        &text,
        start_ms,
        end_ms,
        translation.as_deref(),
    );
    let message = item.message.clone();

    update_shell_snapshot(app, |shell| {
        shell.status_text = message.clone();
        shell.transition_count += 1;
    })?;

    Ok(RecordingBatchResult {
        status: batch_status.into(),
        message,
        items: vec![item],
        bootstrap: build_app_bootstrap(app)?,
    })
}

#[cfg(test)]
mod tests {
    use super::{format_ffmpeg_timestamp, slice_ffmpeg_args};

    #[test]
    fn formats_millisecond_offsets_as_padded_seconds() {
        assert_eq!(format_ffmpeg_timestamp(0), "0.000");
        assert_eq!(format_ffmpeg_timestamp(250), "0.250");
        assert_eq!(format_ffmpeg_timestamp(1500), "1.500");
        assert_eq!(format_ffmpeg_timestamp(60123), "60.123");
    }

    #[test]
    fn slice_args_pad_the_window_and_order_seek_before_input() {
        let args = slice_ffmpeg_args(1000, 2000, "in.wav", "out.mp3");

        let ss = args.iter().position(|arg| arg == "-ss").expect("-ss present");
        let to = args.iter().position(|arg| arg == "-to").expect("-to present");
        let input = args.iter().position(|arg| arg == "-i").expect("-i present");

        // Seek flags must precede the input for keyframe-accurate seeking.
        assert!(ss < input);
        assert!(to < input);

        // 250ms of padding on each side, clamped by saturating math.
        assert_eq!(args[ss + 1], "0.750");
        assert_eq!(args[to + 1], "2.250");

        assert_eq!(args.last().map(String::as_str), Some("out.mp3"));
        assert!(args.iter().any(|arg| arg == "libmp3lame"));
        assert!(args.iter().any(|arg| arg == "128k"));
    }

    #[test]
    fn slice_args_clamp_padding_at_the_start_of_the_file() {
        let args = slice_ffmpeg_args(100, 500, "in.wav", "out.mp3");
        let ss = args.iter().position(|arg| arg == "-ss").expect("-ss present");
        // 100ms - 250ms padding saturates to the start of the file.
        assert_eq!(args[ss + 1], "0.000");
    }
}
