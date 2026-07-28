use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc, Condvar, Mutex},
    thread::JoinHandle,
};

use serde::{Deserialize, Serialize};

use crate::{app_config::RECOMMENDED_WHISPER_RUNTIME_VERSION, recording::RecordingCaptureResult};

pub(crate) const START_SHORTCUT: &str = "Ctrl+Alt+R";
pub(crate) const STOP_SHORTCUT: &str = "Ctrl+Alt+S";
pub(crate) const SHOW_SHORTCUT: &str = "Ctrl+Alt+W";
/// Mines the line playing in a watch session. Global on purpose: it has to fire while
/// mpv has focus, which is the whole point — mining should not mean leaving the video.
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

/// whisper.cpp's built-in Silero VAD ggml model, used for drift-free speech segmentation.
/// Tiny (~0.85 MB); lives alongside the ggml Whisper models under `{asset}/models/`.
pub(crate) const WHISPER_VAD_MODEL_FILE: &str = "ggml-silero-v6.2.0.bin";
pub(crate) const WHISPER_VAD_MODEL_URL: &str =
    "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v6.2.0.bin";

pub(crate) fn default_whisper_model_id() -> &'static str {
    "small"
}

pub(crate) fn default_whisper_model_choice() -> String {
    default_whisper_model_id().to_string()
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

/// Where the global recording-indicator toast is anchored on the primary
/// monitor. One of the six values `normalize_indicator_position` accepts; the
/// centered top edge is the original, most eye-catching placement.
pub(crate) fn default_indicator_position() -> String {
    "top-center".into()
}

/// Matches the browser extension's default provider id (`KNOWN_TRANSLATION_PROVIDERS`
/// in the extension). Sent verbatim in each translation job; the extension routes
/// on it, so the string must stay in lockstep with the extension's ids.
pub(crate) fn default_translation_provider() -> String {
    "google-translate".into()
}

/// English, matching what every translation written before the target language was
/// configurable used. Also the fallback whenever a stored code is unusable, so a
/// broken setting degrades to the old behaviour instead of a broken provider URL.
pub(crate) fn default_translation_target_language() -> String {
    "en".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TranslationSettings {
    /// Which provider the extension should use for desktop-initiated translations:
    /// `"google-translate"` or `"deepl"`. Passed through the bridge as the job's
    /// `provider`; an unknown value simply lets the extension fall back to its own
    /// selection.
    #[serde(default = "default_translation_provider")]
    pub(crate) provider: String,
    /// The language transcripts are translated INTO, as a lowercase ISO 639-1 code.
    /// Sent as the job's `targetLang` and used to name the `{stem}.translation.{lang}.txt`
    /// sidecar. The UI owns which codes are offered; see
    /// `normalize_translation_target_language` for why only the format is enforced here.
    #[serde(default = "default_translation_target_language")]
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

/// Shift, matching Yomitan — the gesture users of this workflow already have in their
/// fingers. It is also the mechanism: over the video, holding the modifier is exactly what
/// stops the overlay being click-through, so the key and the hit-testing are one thing.
pub(crate) fn default_scan_modifier() -> String {
    "shift".into()
}

/// `remainOpen` — releasing the modifier leaves the popup up so it can be read and
/// scrolled. Matches the add-on's own default.
pub(crate) fn default_scan_release_behavior() -> String {
    "remainOpen".into()
}

/// 20 ms, the add-on's value. It is a floor on how often a lookup may *start*, not a delay
/// before the first one — see the two-stage throttle in the scanner.
pub(crate) fn default_scan_debounce_ms() -> u64 {
    20
}

/// 14 px, the add-on's popup default, so a transplanted stylesheet looks identical.
pub(crate) fn default_lookup_font_size_px() -> u64 {
    14
}

/// 28 px, matching the browser extension's `DEFAULT_FONT_SIZE_PX` for subtitles drawn over
/// video — a size chosen against real playback rather than guessed.
pub(crate) fn default_overlay_font_size_px() -> u64 {
    28
}

/// 17 px, the existing `--reading-base` token. Naming it here keeps the Rust default and
/// the stylesheet from drifting.
pub(crate) fn default_reading_font_size_px() -> u64 {
    17
}

/// The word scanner and the typography it is read at.
///
/// Grouped rather than flattened onto `AppSettings` because these are one feature — but
/// note that a nested group must also gain a merge line in the frontend's `updateSettings`,
/// or a partial update silently wipes its siblings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScannerSettings {
    /// Held to scan: `shift` | `ctrl` | `alt` | `none`. `none` scans on bare hover, which
    /// is why the debounce below matters more in that mode.
    #[serde(default = "default_scan_modifier")]
    pub(crate) modifier: String,
    /// What releasing the modifier does: `remainOpen` | `close`.
    #[serde(default = "default_scan_release_behavior")]
    pub(crate) release_behavior: String,
    #[serde(default = "default_scan_debounce_ms")]
    pub(crate) debounce_ms: u64,
    /// Popup font. Empty means inherit the app's reading font. A free string, deliberately:
    /// the UI owns which families it offers, Rust only bounds the length.
    #[serde(default)]
    pub(crate) font_family: String,
    #[serde(default = "default_lookup_font_size_px")]
    pub(crate) font_size_px: u64,
    /// Draw our own scannable subtitles over mpv instead of mpv's styled ones. **Off by
    /// default**: mpv's `.ass` rendering is what works today and stays the default.
    #[serde(default)]
    pub(crate) overlay_enabled: bool,
    #[serde(default = "default_overlay_font_size_px")]
    pub(crate) overlay_font_size_px: u64,
    /// The app's own reading font, driving `--font-reading`. Empty = the built-in stack.
    #[serde(default)]
    pub(crate) reading_font_family: String,
    /// Drives `--reading-base`, which the transcript rows, the live pane and the watch line
    /// already size themselves from.
    #[serde(default = "default_reading_font_size_px")]
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
#[serde(rename_all = "camelCase")]
pub(crate) struct FeatureSettings {
    pub(crate) transcription: bool,
    #[serde(default)]
    pub(crate) delete_local_audio_after_anki_push: bool,
    #[serde(default)]
    pub(crate) allow_mp3_conversion: bool,
    #[serde(default)]
    pub(crate) auto_add_furigana_after_anki_push: bool,
    /// Translate a transcript as soon as it is created, instead of waiting for the
    /// user to press Translate. Needs the browser extension in App Support mode;
    /// when it is not connected the transcript is still saved and the translation
    /// is simply skipped.
    #[serde(default)]
    pub(crate) translate_after_transcription: bool,
}

impl Default for FeatureSettings {
    fn default() -> Self {
        Self {
            transcription: true,
            delete_local_audio_after_anki_push: false,
            allow_mp3_conversion: false,
            auto_add_furigana_after_anki_push: false,
            translate_after_transcription: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnkiFieldMapping {
    pub(crate) transcription: String,
    #[serde(default)]
    pub(crate) furigana: String,
    pub(crate) audio: String,
    pub(crate) translation: String,
    pub(crate) source_path: String,
    pub(crate) created_at: String,
    /// Target field for a clickable link back to the source (YouTube links deep-link
    /// to the sentence's moment). Empty = unmapped.
    #[serde(default)]
    pub(crate) source_url: String,
    /// Target field for the recording's display title. Empty = unmapped.
    #[serde(default)]
    pub(crate) title: String,
    /// Target field for the sentence's timestamp (H:MM:SS). Empty = unmapped.
    #[serde(default)]
    pub(crate) position: String,
    /// Target field for a still frame from the video at the mined line's moment.
    /// Empty = unmapped, which is also what every source without a video gets.
    /// Receives an `<img src="...">` tag.
    #[serde(default)]
    pub(crate) image: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnkiSettings {
    pub(crate) deck_name: String,
    pub(crate) note_type: String,
    #[serde(default)]
    pub(crate) fields: AnkiFieldMapping,
    /// Milliseconds of audio padding added to each side of a mined sentence clip so it does
    /// not cut the first/last syllable. Clamped to the file start on the low side.
    #[serde(default = "default_clip_padding_ms")]
    pub(crate) clip_padding_ms: u64,
}

impl Default for AnkiSettings {
    fn default() -> Self {
        Self {
            deck_name: String::new(),
            note_type: String::new(),
            fields: AnkiFieldMapping::default(),
            clip_padding_ms: default_clip_padding_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WhisperSettings {
    pub(crate) cli_path: String,
    pub(crate) model_path: String,
    #[serde(default = "default_whisper_runtime_version")]
    pub(crate) runtime_version: String,
    #[serde(default = "default_whisper_model_choice")]
    pub(crate) model_choice: String,
    pub(crate) language: String,
    /// How much of the machine transcription may use: `"low" | "balanced" | "high"`. Maps to
    /// a whisper-cli `-t` thread count via `transcription_thread_count`.
    #[serde(default = "default_cpu_usage")]
    pub(crate) cpu_usage: String,
    /// Audio content mode: `"speech"` (default) or `"music"`. Music skips VAD so a full
    /// song (sung vocals) transcribes; speech keeps the VAD-anchored behaviour.
    #[serde(default = "default_audio_type")]
    pub(crate) audio_type: String,
    /// Decoder search width: `"balanced"` (default) or `"fast"`. Fast drops whisper to
    /// greedy decoding (`-bs 1 -bo 1`), measured 13–23% quicker with lateral quality
    /// differences — kana/kanji choice, punctuation, segment boundaries — rather than
    /// worse ones. There is deliberately no "thorough" setting: a wider beam
    /// (`-bs 8 -bo 8`) was measured against the default on both conversation and sung
    /// vocals and recovered nothing, sometimes choosing a worse reading.
    #[serde(default = "default_decode_speed")]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppSettings {
    pub(crate) output_directory: String,
    pub(crate) asset_directory: String,
    #[serde(default)]
    pub(crate) whisper: WhisperSettings,
    #[serde(default)]
    pub(crate) anki: AnkiSettings,
    #[serde(default)]
    pub(crate) features: FeatureSettings,
    #[serde(default)]
    pub(crate) translation: TranslationSettings,
    #[serde(default)]
    pub(crate) scanner: ScannerSettings,
    /// Jimaku API key (jimaku.cc/account). Flat rather than a nested group on purpose: a
    /// single field cannot hit the trap where a partial update wipes a group's siblings.
    #[serde(default)]
    pub(crate) jimaku_api_key: String,
    #[serde(default = "default_theme_preference")]
    pub(crate) theme: String,
    #[serde(default = "default_indicator_position")]
    pub(crate) indicator_position: String,
    #[serde(default)]
    pub(crate) launch_at_login: bool,
    #[serde(default)]
    pub(crate) start_minimized: bool,
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
    /// Path to the `{stem}.{lang}.segments.json` sidecar beside the audio, when
    /// whisper produced parseable per-segment offsets. `None` for transcripts
    /// created before segments existed or when the json was missing/unparseable.
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
    /// Time-aligned segments for per-sentence playback, resolved from the
    /// transcript's `segments_path` sidecar. Empty when there is no sidecar or it
    /// could not be read/parsed — never a reason to fail the read.
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
    /// How this recording entered the library: `"recording"` (mic capture),
    /// `"import"` (a local file the user brought in), or `None` for entries that
    /// predate the field. Serialized as `source` in `src/types.ts`.
    #[serde(default)]
    pub(crate) source: Option<String>,
    /// The origin URL for a future YouTube/network import. Always `None` today.
    #[serde(default)]
    pub(crate) source_url: Option<String>,
    /// The original file name of an imported file, kept for display when the copy
    /// on disk is renamed/sanitized. `None` for mic recordings.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PersistedData {
    pub(crate) settings: AppSettings,
    pub(crate) recent_recordings: Vec<RecentRecording>,
    pub(crate) untitled_counter: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HotkeyBindings {
    pub(crate) start: String,
    pub(crate) stop: String,
    pub(crate) show_window: String,
    /// Mines the line currently playing in a watch session.
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
    pub(crate) whisper_detection: WhisperDetection,
    pub(crate) ffmpeg_detection: FfmpegDetection,
    pub(crate) ytdlp_detection: YtdlpDetection,
    pub(crate) alass_detection: AlassDetection,
    pub(crate) model_download: ModelDownloadSnapshot,
    pub(crate) log_path: String,
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
    /// What the installed binary reports for itself. `None` when nothing is installed, or
    /// when it answered in a shape we do not recognise — see `version_from_banner`.
    pub(crate) version: Option<String>,
    pub(crate) message: String,
}

impl Default for FfmpegDetection {
    fn default() -> Self {
        Self {
            status: "notFound".into(),
            executable_path: None,
            managed: false,
            version: None,
            message: "Not set up yet. Download it to convert transcribed recordings to MP3."
                .into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AlassDetection {
    pub(crate) status: String,
    pub(crate) executable_path: Option<String>,
    /// What the installed binary reports for itself. `None` when nothing is installed, or
    /// when it answered in a shape we do not recognise — see `version_from_banner`.
    pub(crate) version: Option<String>,
    pub(crate) message: String,
}

impl Default for AlassDetection {
    fn default() -> Self {
        Self {
            status: "notFound".into(),
            executable_path: None,
            version: None,
            message: "Not set up yet. Download it to align out-of-sync subtitles automatically.".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct YtdlpDetection {
    pub(crate) status: String,
    pub(crate) executable_path: Option<String>,
    pub(crate) managed: bool,
    /// What the installed binary reports for itself. `None` when nothing is installed, or
    /// when it answered in a shape we do not recognise — see `version_from_banner`.
    pub(crate) version: Option<String>,
    pub(crate) message: String,
}

impl Default for YtdlpDetection {
    fn default() -> Self {
        Self {
            status: "notFound".into(),
            executable_path: None,
            managed: false,
            version: None,
            message: "Not set up yet. Download it to import audio from YouTube."
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
    pub(crate) kind: String,
    pub(crate) status: String,
    pub(crate) message: String,
    pub(crate) current_version: Option<String>,
    pub(crate) latest_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelDownloadSnapshot {
    pub(crate) kind: Option<String>,
    pub(crate) status: String,
    pub(crate) message: String,
    pub(crate) downloaded_bytes: u64,
    pub(crate) total_bytes: Option<u64>,
    pub(crate) progress_percent: Option<f64>,
    pub(crate) target_path: Option<String>,
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
    pub(crate) fields: Vec<String>,
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
}

pub(crate) struct SharedShellState(pub(crate) Mutex<ShellSnapshot>);
pub(crate) struct SharedPersistedState(pub(crate) Mutex<PersistedData>);
pub(crate) struct WhisperDetectionState(pub(crate) Mutex<WhisperDetection>);
pub(crate) struct ModelDownloadState(pub(crate) Mutex<ModelDownloadSnapshot>);
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
