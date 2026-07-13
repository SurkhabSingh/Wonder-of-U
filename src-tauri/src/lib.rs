mod anki;
mod app_config;
mod app_runtime;
mod app_setup;
mod app_state;
mod app_types;
mod asset_downloads;
mod commands;
mod desktop_shell;
mod recording;
mod recording_library;
mod recording_session;
mod runtime_assets;
mod settings;
mod transcription;
mod translation_bridge;

use app_config::AUTOSTART_ARGUMENT;
use app_runtime::{emit_app_snapshot, setup_error};
use app_setup::initialize_app_state;
use commands::*;
use desktop_shell::{
    acquire_single_instance_or_exit, configure_desktop_shell, mark_main_page_loaded,
    StartupVisibility,
};

use std::sync::Arc;

use tauri::{webview::PageLoadEvent, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _single_instance_guard = acquire_single_instance_or_exit();
    let startup_visibility = Arc::new(StartupVisibility::default());
    let setup_visibility = Arc::clone(&startup_visibility);
    let page_load_visibility = Arc::clone(&startup_visibility);

    tauri::Builder::default()
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .arg(AUTOSTART_ARGUMENT)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(move |app| {
            let startup_warnings = initialize_app_state(app)?;
            configure_desktop_shell(app, &setup_visibility, startup_warnings)
                .map_err(setup_error)?;

            translation_bridge::start_bridge_server(app.handle().clone());

            emit_app_snapshot(app.handle());
            Ok(())
        })
        .on_page_load(move |webview, payload| {
            if webview.label() != "main"
                || payload.event() != PageLoadEvent::Finished
                || payload.url().scheme() == "about"
            {
                return;
            }

            mark_main_page_loaded(webview.window().app_handle(), &page_load_visibility);
        })
        .invoke_handler(tauri::generate_handler![
            get_app_bootstrap,
            download_recommended_whisper_model,
            download_recommended_whisper_runtime,
            download_whisper_runtime_version,
            download_recommended_ffmpeg,
            check_whisper_runtime_update,
            check_whisper_model_update,
            toggle_whisper_model_download_pause,
            cancel_whisper_model_download,
            save_settings,
            start_recording,
            stop_recording,
            load_anki_catalog,
            play_recording,
            read_recording_texts,
            delete_recording,
            delete_recordings,
            push_recordings_to_anki,
            push_recordings_to_anki_deck,
            add_furigana_to_anki,
            translate_recordings,
            transcribe_recordings,
            convert_recordings_to_mp3,
            show_main_window,
            hide_main_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use crate::{
        anki::{
            join_anki_field_parts, preserve_anki_sound_tags, recording_pushed_to_anki_target,
            recording_transcript_supports_furigana,
        },
        app_state::{
            normalize_theme_preference, reconcile_recording_history, sanitize_recording_name,
            unique_wav_path,
        },
        app_types::{AnkiSettings, PersistedData, RecentRecording},
        recording_library::rename_recording_outputs_from_transcript,
    };
    use std::path::Path;

    #[test]
    fn sanitize_recording_name_removes_windows_invalid_chars() {
        assert_eq!(sanitize_recording_name("  lesson:01?*  "), "lesson 01");
    }

    #[test]
    fn unique_wav_path_appends_suffix_when_file_exists() {
        let temp_dir = tempfile::tempdir().unwrap();
        let first = temp_dir.path().join("sample.wav");
        std::fs::write(&first, b"test").unwrap();
        let second = unique_wav_path(temp_dir.path(), "sample");
        assert_eq!(
            second.file_name().unwrap().to_string_lossy(),
            "sample_1.wav"
        );
    }

    #[test]
    fn transcript_renames_use_a_matched_timestamped_pair() {
        let temp_dir = tempfile::tempdir().unwrap();
        let audio_path = temp_dir.path().join("recording.wav");
        let transcript_path = temp_dir.path().join("temporary.txt");
        std::fs::write(&audio_path, b"audio").unwrap();
        std::fs::write(&transcript_path, "same audio").unwrap();

        let (renamed_audio, renamed_transcript) =
            rename_recording_outputs_from_transcript(&audio_path, &transcript_path, 12345).unwrap();

        assert_eq!(
            renamed_audio.file_name().unwrap().to_string_lossy(),
            "same audio_12345.wav"
        );
        assert_eq!(
            renamed_transcript.file_name().unwrap().to_string_lossy(),
            "same audio_12345.transcript.txt"
        );
    }

    #[test]
    fn repeated_transcripts_never_reuse_an_existing_output_pair() {
        let temp_dir = tempfile::tempdir().unwrap();

        let first_audio = temp_dir.path().join("recording_a.wav");
        let first_transcript = temp_dir.path().join("temporary_a.txt");
        std::fs::write(&first_audio, b"audio").unwrap();
        std::fs::write(&first_transcript, "same audio").unwrap();
        let first_pair =
            rename_recording_outputs_from_transcript(&first_audio, &first_transcript, 12345)
                .unwrap();

        let second_audio = temp_dir.path().join("recording_b.wav");
        let second_transcript = temp_dir.path().join("temporary_b.txt");
        std::fs::write(&second_audio, b"audio").unwrap();
        std::fs::write(&second_transcript, "same audio").unwrap();
        let second_pair =
            rename_recording_outputs_from_transcript(&second_audio, &second_transcript, 12345)
                .unwrap();

        assert_ne!(first_pair, second_pair);
        assert_eq!(
            second_pair.0.file_name().unwrap().to_string_lossy(),
            "same audio_12345_1.wav"
        );
        assert_eq!(
            second_pair.1.file_name().unwrap().to_string_lossy(),
            "same audio_12345_1.transcript.txt"
        );
    }

    #[test]
    fn theme_preference_accepts_known_values_and_rejects_unknown_values() {
        assert_eq!(normalize_theme_preference("light"), "light");
        assert_eq!(normalize_theme_preference(" dark "), "dark");
        assert_eq!(normalize_theme_preference("sepia"), "system");
    }

    #[test]
    fn persisted_data_counter_defaults_to_positive_value() {
        let state = PersistedData {
            settings: serde_json::from_value(serde_json::json!({
                "outputDirectory": "C:\\Temp",
                "assetDirectory": "C:\\Temp\\assets",
                "whisper": {
                    "cliPath": "",
                    "modelPath": "",
                    "language": "auto"
                },
                "features": {
                    "transcription": true
                },
                "launchAtLogin": false,
                "startMinimized": false
            }))
            .unwrap(),
            recent_recordings: Vec::new(),
            untitled_counter: 0,
        };

        assert_eq!(state.untitled_counter, 0);
        assert_eq!(state.settings.theme, "system");
        assert!(Path::new("C:\\Temp").is_absolute());
    }

    #[test]
    fn recording_history_recovers_untracked_audio_without_dropping_existing_entries() {
        let temp_dir = tempfile::tempdir().unwrap();
        let recovered_audio = temp_dir.path().join("recovered.wav");
        let recovered_transcript = temp_dir.path().join("recovered.transcript.txt");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&recovered_audio, spec).unwrap();
        for _ in 0..16_000 {
            writer.write_sample(0i16).unwrap();
        }
        writer.finalize().unwrap();
        std::fs::write(&recovered_transcript, "recovered transcript").unwrap();

        let existing = RecentRecording {
            file_name: "existing.wav".into(),
            file_path: temp_dir.path().join("existing.wav").display().to_string(),
            transcript_path: None,
            transcript_language: None,
            transcripts: Vec::new(),
            translation_path: None,
            anki_note_id: Some(42),
            anki_deck_name: Some("Japanese".into()),
            anki_note_type: Some("Mining".into()),
            anki_pushes: Vec::new(),
            furigana_applied: false,
            audio_deleted: true,
            duration_ms: 123,
            bytes_written: 0,
            created_at_ms: 1,
        };
        let mut state = PersistedData {
            settings: serde_json::from_value(serde_json::json!({
                "outputDirectory": temp_dir.path().display().to_string(),
                "assetDirectory": temp_dir.path().join("assets").display().to_string(),
                "whisper": {
                    "cliPath": "",
                    "modelPath": "",
                    "language": "auto"
                },
                "features": {
                    "transcription": true
                },
                "launchAtLogin": false,
                "startMinimized": false
            }))
            .unwrap(),
            recent_recordings: vec![existing],
            untitled_counter: 1,
        };

        reconcile_recording_history(&mut state);

        assert_eq!(state.recent_recordings.len(), 2);
        let preserved = state
            .recent_recordings
            .iter()
            .find(|recording| recording.anki_note_id == Some(42))
            .unwrap();
        assert!(preserved.audio_deleted);

        let recovered = state
            .recent_recordings
            .iter()
            .find(|recording| recording.file_name == "recovered.wav")
            .unwrap();
        let recovered_transcript_path = recovered_transcript.display().to_string();
        assert_eq!(
            recovered.transcript_path.as_deref(),
            Some(recovered_transcript_path.as_str())
        );
        assert_eq!(recovered.duration_ms, 1000);
        assert!(recovered.bytes_written > 0);
    }

    #[test]
    fn anki_target_match_requires_same_deck_and_note_type() {
        let settings = AnkiSettings {
            deck_name: "Japanese".into(),
            note_type: "Mining".into(),
            ..Default::default()
        };
        let mut recording = RecentRecording {
            file_name: "sample.wav".into(),
            file_path: "C:\\Temp\\sample.wav".into(),
            transcript_path: Some("C:\\Temp\\sample.transcript.txt".into()),
            transcript_language: Some("ja".into()),
            transcripts: Vec::new(),
            translation_path: None,
            anki_note_id: Some(42),
            anki_deck_name: Some("Japanese".into()),
            anki_note_type: Some("Mining".into()),
            anki_pushes: Vec::new(),
            furigana_applied: false,
            audio_deleted: false,
            duration_ms: 1,
            bytes_written: 1,
            created_at_ms: 1,
        };

        assert!(recording_pushed_to_anki_target(&recording, &settings, "ja"));

        recording.anki_deck_name = Some("Other".into());
        assert!(!recording_pushed_to_anki_target(
            &recording, &settings, "ja"
        ));

        recording.anki_deck_name = Some("Japanese".into());
        recording.anki_note_type = Some("Basic".into());
        assert!(!recording_pushed_to_anki_target(
            &recording, &settings, "ja"
        ));

        recording.anki_note_type = Some("Mining".into());
        recording.anki_note_id = None;
        assert!(!recording_pushed_to_anki_target(
            &recording, &settings, "ja"
        ));
    }

    #[test]
    fn furigana_requires_japanese_transcript_language() {
        let mut recording = RecentRecording {
            file_name: "sample.wav".into(),
            file_path: "C:\\Temp\\sample.wav".into(),
            transcript_path: Some("C:\\Temp\\sample.transcript.txt".into()),
            transcript_language: Some("en".into()),
            transcripts: Vec::new(),
            translation_path: None,
            anki_note_id: Some(42),
            anki_deck_name: Some("Japanese".into()),
            anki_note_type: Some("Mining".into()),
            anki_pushes: Vec::new(),
            furigana_applied: false,
            audio_deleted: false,
            duration_ms: 1,
            bytes_written: 1,
            created_at_ms: 1,
        };

        assert!(!recording_transcript_supports_furigana(
            &recording,
            "日本語を食べる"
        ));

        recording.transcript_language = Some("ja".into());
        assert!(recording_transcript_supports_furigana(
            &recording,
            "plain text"
        ));

        recording.transcript_language = None;
        assert!(recording_transcript_supports_furigana(
            &recording,
            "日本語を食べる"
        ));
        assert!(!recording_transcript_supports_furigana(
            &recording,
            "plain text"
        ));
    }

    #[test]
    fn anki_field_parts_join_without_erasing_audio() {
        assert_eq!(
            join_anki_field_parts("[sound:sample.wav]", "transcript"),
            "[sound:sample.wav]<br>transcript"
        );
        assert_eq!(join_anki_field_parts("", "transcript"), "transcript");
        assert_eq!(
            join_anki_field_parts("[sound:sample.wav]", ""),
            "[sound:sample.wav]"
        );
    }

    #[test]
    fn furigana_replacement_preserves_sound_tags() {
        let result = preserve_anki_sound_tags(
            Some("[sound:sample.wav]<br>old text"),
            "<ruby>text<rt>reading</rt></ruby>",
            None,
        );
        assert_eq!(
            result,
            "[sound:sample.wav]<br><ruby>text<rt>reading</rt></ruby>"
        );
    }

    #[test]
    fn furigana_replacement_uses_fallback_sound_tag() {
        let result = preserve_anki_sound_tags(
            Some("old text"),
            "<ruby>text<rt>reading</rt></ruby>",
            Some("[sound:sample.wav]"),
        );
        assert_eq!(
            result,
            "[sound:sample.wav]<br><ruby>text<rt>reading</rt></ruby>"
        );
    }
}
