use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc, Condvar, Mutex},
    thread::JoinHandle,
};

use serde::{Deserialize, Serialize};

use crate::{
    app_config::RECOMMENDED_WHISPER_RUNTIME_VERSION,
    asset_downloads::{AssetKind, DownloadQueue, QueuedDownload},
    recording::RecordingCaptureResult,
};

pub(crate) const START_SHORTCUT: &str = "Ctrl+Alt+R";
pub(crate) const STOP_SHORTCUT: &str = "Ctrl+Alt+S";
pub(crate) const SHOW_SHORTCUT: &str = "Ctrl+Alt+W";
pub(crate) const MINE_SHORTCUT: &str = "Ctrl+Alt+M";

#[derive(Copy, Clone)]
pub(crate) struct WhisperModelSpec {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) file_name: &'static str,
    pub(crate) download_url: &'static str,
}

pub(crate) const WHISPER_MODEL_SPECS: [WhisperModelSpec; 5] = [
    WhisperModelSpec {
        id: "tiny",
        label: "Tiny",
        file_name: "ggml-tiny.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
    },
    WhisperModelSpec {
        id: "base",
        label: "Base",
        file_name: "ggml-base.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
    },
    WhisperModelSpec {
        id: "small",
        label: "Small",
        file_name: "ggml-small.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
    },
    WhisperModelSpec {
        id: "medium",
        label: "Medium",
        file_name: "ggml-medium.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin",
    },
    WhisperModelSpec {
        id: "large-v3",
        label: "Large v3",
        file_name: "ggml-large-v3.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin",
    },
];

pub(crate) const WHISPER_VAD_MODEL_FILE: &str = "ggml-silero-v6.2.0.bin";

/// Where the VAD model lives, for everyone who needs to know.
pub(crate) fn whisper_vad_model_path(asset_directory: &Path) -> PathBuf {
    asset_directory.join("models").join(WHISPER_VAD_MODEL_FILE)
}
pub(crate) const WHISPER_VAD_MODEL_URL: &str =
    "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v6.2.0.bin";

pub(crate) fn default_whisper_model_id() -> &'static str {
    "small"
}

pub(crate) fn default_whisper_runtime_version() -> String {
    RECOMMENDED_WHISPER_RUNTIME_VERSION.to_string()
}

/// Default CPU-usage preference for transcription: `"balanced"` uses about half the cores.
/// The other accepted values are `"low"` and `"high"` (see `transcription_thread_count`).
fn default_cpu_usage() -> String {
    "balanced".into()
}

fn default_audio_type() -> String {
    "speech".into()
}

fn default_decode_speed() -> String {
    "balanced".into()
}

fn default_clip_padding_ms() -> u64 {
    250
}

pub(crate) fn whisper_model_spec(model_id: &str) -> &'static WhisperModelSpec {
    WHISPER_MODEL_SPECS
        .iter()
        .find(|spec| spec.id == model_id)
        .unwrap_or(&WHISPER_MODEL_SPECS[2])
}

pub(crate) fn default_theme_preference() -> String {
    "system".into()
}

/// Where the global recording-indicator toast is anchored on the primary monitor.
pub(crate) fn default_indicator_position() -> String {
    "top-center".into()
}

pub(crate) fn default_translation_provider() -> String {
    "google-translate".into()
}

pub(crate) fn default_translation_target_language() -> String {
    "en".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct TranslationSettings {
    pub(crate) provider: String,
    pub(crate) target_language: String,
}

impl Default for TranslationSettings {
    fn default() -> Self {
        Self {
            provider: default_translation_provider(),
            target_language: default_translation_target_language(),
        }
    }
}

pub(crate) fn default_scan_modifier() -> String {
    "shift".into()
}

pub(crate) fn default_scan_release_behavior() -> String {
    "remainOpen".into()
}

pub(crate) fn default_scan_debounce_ms() -> u64 {
    20
}

pub(crate) fn default_lookup_font_size_px() -> u64 {
    14
}

pub(crate) fn default_overlay_font_size_px() -> u64 {
    28
}

pub(crate) fn default_reading_font_size_px() -> u64 {
    17
}

/// The word scanner and the typography it is read at.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct ScannerSettings {
    pub(crate) modifier: String,
    pub(crate) release_behavior: String,
    pub(crate) debounce_ms: u64,
    pub(crate) font_family: String,
    pub(crate) font_size_px: u64,
    pub(crate) overlay_enabled: bool,
    pub(crate) overlay_font_size_px: u64,
    pub(crate) reading_font_family: String,
    pub(crate) reading_font_size_px: u64,
}

impl Default for ScannerSettings {
    fn default() -> Self {
        Self {
            modifier: default_scan_modifier(),
            release_behavior: default_scan_release_behavior(),
            debounce_ms: default_scan_debounce_ms(),
            font_family: String::new(),
            font_size_px: default_lookup_font_size_px(),
            overlay_enabled: false,
            overlay_font_size_px: default_overlay_font_size_px(),
            reading_font_family: String::new(),
            reading_font_size_px: default_reading_font_size_px(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct FeatureSettings {
    pub(crate) transcription: bool,
    pub(crate) delete_local_audio_after_anki_push: bool,
    pub(crate) allow_mp3_conversion: bool,
    pub(crate) auto_add_furigana_after_anki_push: bool,
    pub(crate) translate_after_transcription: bool,
    #[serde(default)]
    pub(crate) add_definitions_to_mined_cards: bool,
    #[serde(default)]
    pub(crate) allow_duplicate_mined_words: bool,
    #[serde(default)]
    pub(crate) mine_words_without_context: bool,
}

impl Default for FeatureSettings {
    fn default() -> Self {
        Self {
            transcription: true,
            delete_local_audio_after_anki_push: false,
            allow_mp3_conversion: false,
            auto_add_furigana_after_anki_push: false,
            translate_after_transcription: false,
            add_definitions_to_mined_cards: false,
            allow_duplicate_mined_words: false,
            mine_words_without_context: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct AnkiFieldMapping {
    pub(crate) transcription: String,
    pub(crate) furigana: String,
    pub(crate) audio: String,
    pub(crate) translation: String,
    pub(crate) source_path: String,
    pub(crate) created_at: String,
    pub(crate) source_url: String,
    pub(crate) title: String,
    pub(crate) position: String,
    pub(crate) image: String,
    pub(crate) video: String,
    pub(crate) definition: String,
    pub(crate) word: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct AnkiSettings {
    pub(crate) deck_name: String,
    pub(crate) note_type: String,
    pub(crate) fields: AnkiFieldMapping,
    pub(crate) clip_padding_ms: u64,
    #[serde(default)]
    pub(crate) vocabulary_sources: Vec<VocabularySource>,
    #[serde(default = "default_known_word_interval_days")]
    pub(crate) known_word_interval_days: u32,
    #[serde(default)]
    pub(crate) definition_dictionary_ids: Vec<i64>,
}

impl Default for AnkiSettings {
    fn default() -> Self {
        Self {
            deck_name: String::new(),
            note_type: String::new(),
            fields: AnkiFieldMapping::default(),
            clip_padding_ms: default_clip_padding_ms(),
            vocabulary_sources: Vec::new(),
            known_word_interval_days: default_known_word_interval_days(),
            definition_dictionary_ids: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct WhisperSettings {
    pub(crate) cli_path: String,
    pub(crate) model_path: String,
    pub(crate) runtime_version: String,
    pub(crate) model_choice: String,
    pub(crate) language: String,
    /// How much of the machine transcription may use: `"low" | "balanced" | "high"`. Maps to
    /// a whisper-cli `-t` thread count via `transcription_thread_count`.
    pub(crate) cpu_usage: String,
    pub(crate) audio_type: String,
    /// Decoder search width: `"balanced"` (default) or `"fast"`. Fast drops whisper to
    /// greedy decoding (`-bs 1 -bo 1`), measured 13–23% quicker with lateral quality
    /// differences — kana/kanji choice, punctuation, segment boundaries — rather than
    /// worse ones. There is deliberately no "thorough" setting: a wider beam
    /// (`-bs 8 -bo 8`) was measured against the default on both conversation and sung
    /// vocals and recovered nothing, sometimes choosing a worse reading.
    pub(crate) decode_speed: String,
}

impl Default for WhisperSettings {
    fn default() -> Self {
        Self {
            cli_path: String::new(),
            model_path: String::new(),
            runtime_version: default_whisper_runtime_version(),
            model_choice: default_whisper_model_id().into(),
            language: "auto".into(),
            cpu_usage: default_cpu_usage(),
            audio_type: default_audio_type(),
            decode_speed: default_decode_speed(),
        }
    }
}

/// Every settings struct carries a container-level `default`, so a key missing from
/// `state.json` falls back to that struct's `Default` rather than failing the parse.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct AppSettings {
    pub(crate) output_directory: String,
    pub(crate) asset_directory: String,
    pub(crate) whisper: WhisperSettings,
    pub(crate) anki: AnkiSettings,
    pub(crate) features: FeatureSettings,
    pub(crate) translation: TranslationSettings,
    pub(crate) scanner: ScannerSettings,
    pub(crate) jimaku_api_key: String,
    pub(crate) theme: String,
    pub(crate) indicator_position: String,
    pub(crate) launch_at_login: bool,
    pub(crate) start_minimized: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            output_directory: String::new(),
            asset_directory: String::new(),
            whisper: WhisperSettings::default(),
            anki: AnkiSettings::default(),
            features: FeatureSettings::default(),
            translation: TranslationSettings::default(),
            scanner: ScannerSettings::default(),
            jimaku_api_key: String::new(),
            theme: default_theme_preference(),
            indicator_position: default_indicator_position(),
            launch_at_login: false,
            start_minimized: false,
        }
    }
}

/// One time-aligned sentence/segment parsed from whisper's `--output-json`
/// sidecar, used to drive per-sentence audio playback.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordingSegment {
    pub(crate) text: String,
    pub(crate) start_ms: u64,
    pub(crate) end_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordingTranscript {
    pub(crate) language: String,
    pub(crate) file_path: String,
    #[serde(default)]
    pub(crate) detected_language: Option<String>,
    #[serde(default)]
    pub(crate) segments_path: Option<String>,
}

/// A single transcript or translation text file, resolved for the reader view.
/// `missing` is set (with `text` left empty) when the sidecar could not be read
/// or resolved inside the recording's own folder, so one absent file degrades a
/// pane instead of failing the whole request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordingTextDocument {
    pub(crate) language: String,
    #[serde(default)]
    pub(crate) detected_language: Option<String>,
    pub(crate) file_path: String,
    pub(crate) text: String,
    pub(crate) missing: bool,
    #[serde(default)]
    pub(crate) segments: Vec<RecordingSegment>,
}

/// The full text payload behind the transcript viewer for one recording: every
/// language transcript beside the audio, plus its translation sidecar(s).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordingTexts {
    pub(crate) file_path: String,
    pub(crate) transcripts: Vec<RecordingTextDocument>,
    pub(crate) translations: Vec<RecordingTextDocument>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordingAnkiPush {
    pub(crate) language: String,
    pub(crate) deck_name: String,
    pub(crate) note_type: String,
    pub(crate) note_id: i64,
    #[serde(default)]
    pub(crate) furigana_applied: bool,
}

pub(crate) fn transcript_language_key(language: &str) -> String {
    let key = language.trim().to_ascii_lowercase();
    if key.is_empty() {
        "auto".into()
    } else {
        key
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecentRecording {
    pub(crate) file_name: String,
    pub(crate) file_path: String,
    #[serde(default)]
    pub(crate) transcript_path: Option<String>,
    #[serde(default)]
    pub(crate) transcript_language: Option<String>,
    #[serde(default)]
    pub(crate) transcripts: Vec<RecordingTranscript>,
    #[serde(default)]
    pub(crate) translation_path: Option<String>,
    #[serde(default)]
    pub(crate) anki_note_id: Option<i64>,
    #[serde(default)]
    pub(crate) anki_deck_name: Option<String>,
    #[serde(default)]
    pub(crate) anki_note_type: Option<String>,
    #[serde(default)]
    pub(crate) anki_pushes: Vec<RecordingAnkiPush>,
    #[serde(default)]
    pub(crate) furigana_applied: bool,
    #[serde(default)]
    pub(crate) audio_deleted: bool,
    pub(crate) duration_ms: u64,
    pub(crate) bytes_written: u64,
    pub(crate) created_at_ms: u64,
    #[serde(default)]
    pub(crate) source: Option<String>,
    #[serde(default)]
    pub(crate) source_url: Option<String>,
    #[serde(default)]
    pub(crate) title: Option<String>,
}

impl RecentRecording {
    pub(crate) fn transcript_for_language(&self, language: &str) -> Option<&RecordingTranscript> {
        let key = transcript_language_key(language);
        self.transcripts
            .iter()
            .find(|transcript| transcript.language == key)
    }

    pub(crate) fn has_transcript_for_language(&self, language: &str) -> bool {
        self.transcript_for_language(language).is_some()
    }

    pub(crate) fn transcript_source_for(&self, language: &str) -> Option<(&str, String)> {
        if let Some(variant) = self.transcript_for_language(language) {
            return Some((variant.file_path.as_str(), variant.language.clone()));
        }
        if self.transcripts.is_empty() {
            if let Some(path) = self.transcript_path.as_deref() {
                return Some((
                    path,
                    self.transcript_language
                        .clone()
                        .unwrap_or_else(|| "auto".to_string()),
                ));
            }
        }
        None
    }

    pub(crate) fn anki_push_for_target(
        &self,
        language: &str,
        deck_name: &str,
        note_type: &str,
    ) -> Option<&RecordingAnkiPush> {
        let language = transcript_language_key(language);
        self.anki_pushes.iter().find(|push| {
            push.language == language && push.deck_name == deck_name && push.note_type == note_type
        })
    }
}

/// A video the user has added to the video library, and the subtitle it is paired with.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct WatchedVideo {
    pub(crate) video_path: String,
    pub(crate) title: Option<String>,
    pub(crate) subtitle_path: Option<String>,
    pub(crate) subtitle_origin: Option<String>,
    pub(crate) thumbnail_path: Option<String>,
    pub(crate) duration_ms: u64,
    pub(crate) bytes: u64,
    pub(crate) added_at_ms: u64,
    pub(crate) last_opened_at_ms: Option<u64>,
    pub(crate) resume_position_ms: Option<u64>,
}

/// The whole of `state.json`.
///
/// Carries a container-level `default` for the same reason every settings struct does, and with
/// a heavier consequence. Serde requires every field of a struct unless it can default, so
/// adding a field here would make every state file written before it fail to parse — and an
/// unparseable state file is not a small loss: `load_persisted_data` moves it aside and starts
/// from scratch, taking the recording library and every setting with it. A field added later
/// must be able to be absent, because on the first launch after an update it always is.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct PersistedData {
    pub(crate) settings: AppSettings,
    pub(crate) recent_recordings: Vec<RecentRecording>,
    pub(crate) untitled_counter: u64,
    pub(crate) watched_videos: Vec<WatchedVideo>,
}

impl Default for PersistedData {
    fn default() -> Self {
        Self {
            settings: AppSettings::default(),
            recent_recordings: Vec::new(),
            untitled_counter: 1,
            watched_videos: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HotkeyBindings {
    pub(crate) start: String,
    pub(crate) stop: String,
    pub(crate) show_window: String,
    #[serde(default)]
    pub(crate) mine: String,
}

impl Default for HotkeyBindings {
    fn default() -> Self {
        Self {
            start: START_SHORTCUT.to_string(),
            stop: STOP_SHORTCUT.to_string(),
            show_window: SHOW_SHORTCUT.to_string(),
            mine: MINE_SHORTCUT.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ShellSnapshot {
    pub(crate) phase: String,
    pub(crate) status_text: String,
    pub(crate) last_shortcut: Option<String>,
    pub(crate) transition_count: u32,
    pub(crate) hotkeys: HotkeyBindings,
    pub(crate) started_at_ms: Option<u64>,
    pub(crate) current_recording_name: Option<String>,
    pub(crate) last_output_path: Option<String>,
    pub(crate) last_transcript_path: Option<String>,
}

impl Default for ShellSnapshot {
    fn default() -> Self {
        Self {
            phase: "idle".into(),
            status_text: "Tray shell is ready. Press Ctrl+Alt+R to start recording system audio."
                .into(),
            last_shortcut: None,
            transition_count: 0,
            hotkeys: HotkeyBindings::default(),
            started_at_ms: None,
            current_recording_name: None,
            last_output_path: None,
            last_transcript_path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppBootstrap {
    pub(crate) shell: ShellSnapshot,
    pub(crate) settings: AppSettings,
    pub(crate) recent_recordings: Vec<RecentRecording>,
    pub(crate) watched_videos: Vec<WatchedVideo>,
    pub(crate) whisper_detection: WhisperDetection,
    pub(crate) ffmpeg_detection: FfmpegDetection,
    pub(crate) ytdlp_detection: YtdlpDetection,
    pub(crate) alass_detection: AlassDetection,
    pub(crate) mpv_detection: MpvDetection,
    pub(crate) model_download: ModelDownloadSnapshot,
    pub(crate) dictionary_detection: DictionaryDetection,
    pub(crate) known_words: KnownWordsSnapshot,
    pub(crate) transcription_requirements: Vec<TranscriptionRequirement>,
    pub(crate) log_path: String,
    pub(crate) logging_failure: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DictionaryDetection {
    pub(crate) status: String,
    pub(crate) dictionary_path: Option<String>,
    pub(crate) managed: bool,
    pub(crate) message: String,
}

/// Not-installed is the resting state, so it is what `Default` means here.
impl Default for DictionaryDetection {
    fn default() -> Self {
        Self {
            status: "notFound".into(),
            dictionary_path: None,
            managed: false,
            message:
                "Install the Japanese dictionary to analyse transcript sentences word by word."
                    .into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WhisperDetection {
    pub(crate) status: String,
    pub(crate) executable_path: Option<String>,
    pub(crate) model_path: Option<String>,
    pub(crate) source: Option<String>,
    pub(crate) model_source: Option<String>,
    pub(crate) runtime_version: String,
    pub(crate) available_runtime_versions: Vec<String>,
    pub(crate) cli_ready: bool,
    pub(crate) model_ready: bool,
    pub(crate) vad_ready: bool,
    pub(crate) cli_managed: bool,
    pub(crate) model_managed: bool,
    pub(crate) message: String,
}

impl Default for WhisperDetection {
    fn default() -> Self {
        Self {
            status: "notFound".into(),
            executable_path: None,
            model_path: None,
            source: None,
            model_source: None,
            runtime_version: default_whisper_runtime_version(),
            available_runtime_versions: Vec::new(),
            cli_ready: false,
            model_ready: false,
            vad_ready: false,
            cli_managed: false,
            model_managed: false,
            message:
                "Add or download whisper-cli and a Whisper model to enable offline transcription."
                    .into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FfmpegDetection {
    pub(crate) status: String,
    pub(crate) executable_path: Option<String>,
    pub(crate) managed: bool,
    pub(crate) message: String,
}

impl Default for FfmpegDetection {
    fn default() -> Self {
        Self {
            status: "notFound".into(),
            executable_path: None,
            managed: false,
            message: "Install app-managed FFmpeg to manually convert transcribed WAV recordings into MP3."
                .into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AlassDetection {
    pub(crate) status: String,
    pub(crate) executable_path: Option<String>,
    pub(crate) message: String,
}

impl Default for AlassDetection {
    fn default() -> Self {
        Self {
            status: "notFound".into(),
            executable_path: None,
            message:
                "alass is not installed. Download it to align out-of-sync subtitles automatically."
                    .into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct YtdlpDetection {
    pub(crate) status: String,
    pub(crate) executable_path: Option<String>,
    pub(crate) managed: bool,
    pub(crate) message: String,
}

impl Default for YtdlpDetection {
    fn default() -> Self {
        Self {
            status: "notFound".into(),
            executable_path: None,
            managed: false,
            message: "Install app-managed yt-dlp to import audio from a link."
                .into(),
        }
    }
}

/// Whether an mpv the app can drive is available. Same shape as the other managed
/// binaries, but detection prefers a user's own install — see `detect_local_mpv`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MpvDetection {
    pub(crate) status: String,
    pub(crate) executable_path: Option<String>,
    pub(crate) managed: bool,
    pub(crate) message: String,
}

impl Default for MpvDetection {
    fn default() -> Self {
        Self {
            status: "notFound".into(),
            executable_path: None,
            managed: false,
            message: "Install mpv to watch a video and mine lines as you go.".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WhisperAssetUpdateResult {
    pub(crate) kind: AssetKind,
    pub(crate) status: String,
    pub(crate) message: String,
    pub(crate) current_version: Option<String>,
    pub(crate) latest_version: Option<String>,
}

/// One thing transcription cannot run without.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TranscriptionRequirement {
    pub(crate) id: &'static str,
    pub(crate) ready: bool,
    #[serde(skip)]
    pub(crate) blocked_message: String,
    #[serde(skip)]
    pub(crate) fixed_by: Vec<QueuedDownload>,
}

impl TranscriptionRequirement {
    pub(crate) fn new(
        id: &'static str,
        ready: bool,
        blocked_message: String,
        fixed_by: Vec<QueuedDownload>,
    ) -> Self {
        Self {
            id,
            ready,
            blocked_message,
            fixed_by: if ready { Vec::new() } else { fixed_by },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelDownloadSnapshot {
    pub(crate) kind: Option<AssetKind>,
    pub(crate) status: String,
    pub(crate) message: String,
    pub(crate) downloaded_bytes: u64,
    pub(crate) total_bytes: Option<u64>,
    pub(crate) progress_percent: Option<f64>,
    pub(crate) target_path: Option<String>,
    pub(crate) queued_remaining: usize,
}

impl Default for ModelDownloadSnapshot {
    fn default() -> Self {
        Self {
            kind: None,
            status: "idle".into(),
            message: "No download in progress.".into(),
            downloaded_bytes: 0,
            total_bytes: None,
            progress_percent: None,
            target_path: None,
            queued_remaining: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnkiCatalog {
    pub(crate) status: String,
    pub(crate) message: String,
    pub(crate) version: Option<i64>,
    pub(crate) decks: Vec<String>,
    pub(crate) note_types: Vec<String>,
    pub(crate) note_type: String,
    pub(crate) fields: Option<Vec<String>>,
}

/// Sentences already present in the Anki mining destination, so the transcript viewer
/// can mark them without the user having to remember. `status` is "ready", "offline",
/// or "unmapped" — the last two carry an empty list and are a normal state, not a
/// failure: a transcript still opens fine when Anki is closed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MinedSentences {
    pub(crate) status: String,
    pub(crate) message: String,
    pub(crate) sentences: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordingActionItem {
    pub(crate) file_path: String,
    pub(crate) status: String,
    pub(crate) message: String,
    pub(crate) note_id: Option<i64>,
}

/// One line handed to a batch mine.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MineLineRequest {
    pub(crate) text: String,
    pub(crate) start_ms: u64,
    pub(crate) end_ms: u64,
    pub(crate) translation: Option<String>,
}

/// What became of one line in a batch.
///
/// Carries the line itself rather than the recording path every item would share.
/// A batch reporting "3 of 35 failed" and leaving the reader to work out WHICH
/// three is a batch that has to be redone from the top.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MinedLineOutcome {
    pub(crate) text: String,
    pub(crate) start_ms: u64,
    pub(crate) end_ms: u64,
    pub(crate) status: String,
    pub(crate) message: String,
}

/// The result of mining several lines at once.
///
/// Every line handed in comes back, successes included: the caller needs them to
/// mark the rows it just mined, and the failures are only meaningful next to what
/// did work.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MinedLinesResult {
    pub(crate) status: String,
    pub(crate) message: String,
    pub(crate) added: usize,
    pub(crate) failed: usize,
    pub(crate) lines: Vec<MinedLineOutcome>,
    pub(crate) bootstrap: AppBootstrap,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordingBatchResult {
    pub(crate) status: String,
    pub(crate) message: String,
    pub(crate) items: Vec<RecordingActionItem>,
    pub(crate) bootstrap: AppBootstrap,
}

#[derive(Clone)]
pub(crate) struct AppPathsState {
    pub(crate) data_dir: PathBuf,
    pub(crate) state_file: PathBuf,
    pub(crate) log_file: PathBuf,
    pub(crate) assets_dir: PathBuf,
    pub(crate) known_words_file: PathBuf,
    pub(crate) progress_file: PathBuf,
}

pub(crate) struct SharedShellState(pub(crate) Mutex<ShellSnapshot>);
pub(crate) struct SharedPersistedState(pub(crate) Mutex<PersistedData>);
pub(crate) struct WhisperDetectionState(pub(crate) Mutex<WhisperDetection>);
pub(crate) struct ModelDownloadState(pub(crate) Mutex<ModelDownloadSnapshot>);
pub(crate) struct ModelDownloadQueueState(pub(crate) Mutex<DownloadQueue>);
pub(crate) struct ModelDownloadControlState {
    pub(crate) control: Mutex<ModelDownloadControl>,
    pub(crate) condvar: Condvar,
}
pub(crate) struct RecorderState(pub(crate) Mutex<Option<ActiveRecording>>);

#[derive(Default)]
pub(crate) struct ModelDownloadControl {
    pub(crate) active: bool,
    pub(crate) paused: bool,
    pub(crate) cancel_requested: bool,
}

pub(crate) struct ActiveRecording {
    pub(crate) stop_signal: Arc<AtomicBool>,
    pub(crate) worker: JoinHandle<Result<RecordingCaptureResult, String>>,
}

#[cfg(test)]
mod settings_default_tests {
    use super::*;

    /// The defaults, written out by hand rather than read back from `Default`.
    #[test]
    fn an_empty_settings_object_deserializes_to_the_documented_defaults() {
        let settings: AppSettings = serde_json::from_str("{}").expect("empty settings parse");

        assert_eq!(settings.output_directory, "");
        assert_eq!(settings.asset_directory, "");
        assert_eq!(settings.jimaku_api_key, "");
        assert_eq!(settings.theme, "system");
        assert_eq!(settings.indicator_position, "top-center");
        assert!(!settings.launch_at_login);
        assert!(!settings.start_minimized);

        assert_eq!(settings.whisper.cli_path, "");
        assert_eq!(settings.whisper.model_path, "");
        assert_eq!(settings.whisper.runtime_version, "v1.8.4");
        assert_eq!(settings.whisper.model_choice, "small");
        assert_eq!(settings.whisper.language, "auto");
        assert_eq!(settings.whisper.cpu_usage, "balanced");
        assert_eq!(settings.whisper.audio_type, "speech");
        assert_eq!(settings.whisper.decode_speed, "balanced");

        assert_eq!(settings.anki.deck_name, "");
        assert_eq!(settings.anki.note_type, "");
        assert_eq!(settings.anki.clip_padding_ms, 250);
        assert_eq!(settings.anki.fields.transcription, "");
        assert_eq!(settings.anki.fields.video, "");

        // The one default that is not the type's own zero value: transcription is the
        // app's whole purpose, so absence must not read as "off".
        assert!(settings.features.transcription);
        assert!(!settings.features.delete_local_audio_after_anki_push);
        assert!(!settings.features.allow_mp3_conversion);
        assert!(!settings.features.auto_add_furigana_after_anki_push);
        assert!(!settings.features.translate_after_transcription);

        assert_eq!(settings.translation.provider, "google-translate");
        assert_eq!(settings.translation.target_language, "en");

        assert_eq!(settings.scanner.modifier, "shift");
        assert_eq!(settings.scanner.release_behavior, "remainOpen");
        assert_eq!(settings.scanner.debounce_ms, 20);
        assert_eq!(settings.scanner.font_family, "");
        assert_eq!(settings.scanner.font_size_px, 14);
        assert!(!settings.scanner.overlay_enabled);
        assert_eq!(settings.scanner.overlay_font_size_px, 28);
        assert_eq!(settings.scanner.reading_font_family, "");
        assert_eq!(settings.scanner.reading_font_size_px, 17);
    }

    /// The case that cost the whole library: a key deleted by hand from a group that
    /// still has other keys in it. The group used to fail to parse, and with it the file.
    #[test]
    fn deleting_one_key_by_hand_keeps_the_rest_of_the_group() {
        let raw = r#"{
            "features": { "allowMp3Conversion": true },
            "anki": { "deckName": "Mining", "fields": { "video": "Video" } },
            "whisper": { "language": "ja" }
        }"#;
        let settings: AppSettings = serde_json::from_str(raw).expect("partial settings parse");

        assert!(settings.features.allow_mp3_conversion);
        assert!(
            settings.features.transcription,
            "absent means default, not false"
        );
        assert_eq!(settings.anki.deck_name, "Mining");
        assert_eq!(settings.anki.fields.video, "Video");
        assert_eq!(settings.anki.fields.audio, "");
        assert_eq!(settings.anki.clip_padding_ms, 250);
        assert_eq!(settings.whisper.language, "ja");
        assert_eq!(settings.whisper.model_choice, "small");
    }

    /// A round trip has to survive, or the tolerance above would be hiding a rename.
    /// Today's exact shape keeps loading, with its contents intact.
    ///
    /// A sanity check rather than the regression guard — it supplies all three fields, so it
    /// would pass with or without the container default. The guard is the test below.
    #[test]
    fn a_state_file_without_a_newly_added_field_still_loads() {
        // Exactly the shape written today, plus one recording so the loss would be visible.
        let raw = r#"{
            "settings": { "theme": "dark", "anki": { "deckName": "Mining" } },
            "recentRecordings": [{
                "fileName": "a.wav",
                "filePath": "/rec/a.wav",
                "durationMs": 1000,
                "bytesWritten": 32000,
                "createdAtMs": 1700000000000
            }],
            "untitledCounter": 7
        }"#;

        let state: PersistedData = serde_json::from_str(raw).expect("an older state file loads");

        assert_eq!(state.recent_recordings.len(), 1, "the library survives");
        assert_eq!(state.recent_recordings[0].file_name, "a.wav");
        assert_eq!(state.settings.theme, "dark", "settings survive");
        assert_eq!(state.settings.anki.deck_name, "Mining");
        assert_eq!(state.untitled_counter, 7);
    }

    /// **The regression guard.** Every field absent is what a state file looks like to a build
    /// that has since gained one, so this is the case that decides whether adding a field is
    /// safe. Without the container `default` it fails with "missing field `settings`" — and in
    /// production that failure is not an error the user sees, it is `load_persisted_data`
    /// moving the file aside and starting fresh, taking the recording library with it.
    ///
    /// Verified by removing the attribute and watching this fail.
    #[test]
    fn an_empty_state_file_is_a_usable_first_run() {
        let state: PersistedData = serde_json::from_str("{}").expect("an empty object loads");

        assert!(state.recent_recordings.is_empty());
        // Never 0: this counter names recordings, and "Untitled 0" is not a name.
        assert_eq!(state.untitled_counter, 1);
        assert!(state.settings.features.transcription);
    }

    #[test]
    fn settings_survive_a_round_trip_through_json() {
        let mut settings = AppSettings::default();
        settings.anki.fields.image = "Screenshot".into();
        settings.scanner.overlay_enabled = true;
        settings.whisper.decode_speed = "fast".into();

        let encoded = serde_json::to_string(&settings).expect("encode");
        let decoded: AppSettings = serde_json::from_str(&encoded).expect("decode");

        assert_eq!(decoded.anki.fields.image, "Screenshot");
        assert!(decoded.scanner.overlay_enabled);
        assert_eq!(decoded.whisper.decode_speed, "fast");
        assert!(decoded.features.transcription);
    }
}

/// One note type + field the known-word index is read from.
///
/// Named explicitly rather than inferred from a deck or from "the first field":
/// a deck is a study schedule, not a vocabulary list, and a first field is as
/// often a sentence or an id as it is a word. Independent of the push `note_type`
/// on purpose — the note type cards are pushed INTO is rarely one vocabulary is
/// read FROM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VocabularySource {
    pub(crate) note_type: String,
    pub(crate) field: String,
}

/// What one transcript line asks of the reader.
///
/// `unknown_words` rather than a bare count, because the count alone leaves the
/// user to work out WHICH word is new — and on an i+1 line, that one word is the
/// entire reason to mine it.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LineRanking {
    pub(crate) unknown_words: Vec<String>,
    pub(crate) content_word_count: usize,
    pub(crate) within_reach: bool,
    pub(crate) has_context: bool,
}

/// A ranking of every line handed in, in the same order.
///
/// `lines` always has one entry per input line, whatever `status` says. A short
/// list on the unhappy paths would be a second shape for every caller to handle,
/// and the one they forget.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TranscriptRanking {
    pub(crate) status: String,
    pub(crate) message: String,
    pub(crate) lines: Vec<LineRanking>,
}

/// One proposed vocabulary source, with real values from the user's own cards.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VocabularySuggestion {
    pub(crate) note_type: String,
    pub(crate) field: String,
    pub(crate) mature_note_count: usize,
    pub(crate) samples: Vec<String>,
    pub(crate) already_added: bool,
}

/// The result of looking through the collection. `status` is `ready`, `none`
/// (nothing read like vocabulary), `offline`, or `needsDictionary`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VocabularySuggestions {
    pub(crate) status: String,
    pub(crate) message: String,
    pub(crate) suggestions: Vec<VocabularySuggestion>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KnownWordsSnapshot {
    pub(crate) status: String,
    pub(crate) message: String,
    pub(crate) word_count: usize,
    pub(crate) built_at_ms: Option<u64>,
}

/// Everything that decides what an index contains: which fields the words are read
/// from, and how long one must have been held before it counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KnownWordsBuild {
    #[serde(default)]
    pub(crate) sources: Vec<VocabularySource>,
    #[serde(default)]
    pub(crate) mature_after_days: u32,
}

impl KnownWordsBuild {
    pub(crate) fn from_anki_settings(anki: &AnkiSettings) -> Self {
        Self {
            sources: anki
                .vocabulary_sources
                .iter()
                .filter(|source| !source.note_type.is_empty() && !source.field.is_empty())
                .cloned()
                .collect(),
            mature_after_days: anki.known_word_interval_days,
        }
    }

    pub(crate) fn matches(&self, other: &KnownWordsBuild) -> bool {
        if self.mature_after_days != other.mature_after_days
            || self.sources.len() != other.sources.len()
        {
            return false;
        }
        let key = |sources: &[VocabularySource]| {
            let mut rows: Vec<(String, String)> = sources
                .iter()
                .map(|source| (source.note_type.clone(), source.field.clone()))
                .collect();
            rows.sort_unstable();
            rows
        };
        key(&self.sources) == key(&other.sources)
    }
}

pub(crate) struct KnownWordIndex {
    pub(crate) words: std::collections::HashSet<String>,
    pub(crate) built_at_ms: u64,
    pub(crate) build: KnownWordsBuild,
}

/// The in-memory known-word index, or `None` until one is built or loaded.
pub(crate) struct KnownWordsState(pub(crate) Mutex<Option<KnownWordIndex>>);

/// Anki's own "mature" threshold, and the default both MorphMan and AnkiMorphs use.
pub(crate) fn default_known_word_interval_days() -> u32 {
    21
}
