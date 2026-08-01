export type RecorderPhase =
  | "idle"
  | "recording"
  | "saving"
  | "transcribing"
  | "error"
  | string;

export type HotkeyBindings = {
  start: string;
  stop: string;
  showWindow: string;
  // Mines the line playing in a watch session. Global so it fires while mpv has
  // focus — mining should not mean leaving the video.
  mine: string;
};

export type ShellSnapshot = {
  phase: RecorderPhase;
  statusText: string;
  lastShortcut: string | null;
  transitionCount: number;
  hotkeys: HotkeyBindings;
  startedAtMs: number | null;
  currentRecordingName: string | null;
  lastOutputPath: string | null;
  lastTranscriptPath: string | null;
};

export type FeatureSettings = {
  transcription: boolean;
  deleteLocalAudioAfterAnkiPush: boolean;
  allowMp3Conversion: boolean;
  autoAddFuriganaAfterAnkiPush: boolean;
  translateAfterTranscription: boolean;
};

export type AnkiFieldMapping = {
  transcription: string;
  furigana: string;
  audio: string;
  translation: string;
  sourcePath: string;
  createdAt: string;
  sourceUrl: string;
  title: string;
  position: string;
  // Receives an <img> of a still grabbed from the video at the mined line's moment.
  // Empty = unmapped, which is also every source that has no video.
  image: string;
  // Receives a [sound:...] tag for a short video of the line. Anki renders a video behind
  // that tag as a player, and treats it as media it owns, so it works on the phone clients
  // and Check Media counts it. Empty = unmapped, which is what turns clip capture off.
  video: string;
};

export type AnkiSettings = {
  deckName: string;
  noteType: string;
  fields: AnkiFieldMapping;
  // Milliseconds of audio padding added to each side of a mined sentence clip.
  clipPaddingMs: number;
  // Which note types and fields hold the words you already know. Independent of
  // `noteType` above: the notes cards are pushed INTO are rarely the ones your
  // vocabulary is read FROM.
  vocabularySources: VocabularySource[];
  // How long a word has to have stuck before it counts as known, in days. 21 is
  // Anki's own "mature" line.
  knownWordIntervalDays: number;
};

export type VocabularySource = {
  noteType: string;
  field: string;
};

export type WhisperSettings = {
  cliPath: string;
  modelPath: string;
  runtimeVersion: string;
  modelChoice: string;
  language: string;
  // How much of the machine transcription may use: "low" | "balanced" | "high".
  // The backend maps it to a whisper-cli thread count; "balanced" is the default.
  cpuUsage: string;
  // Audio content mode: "speech" (default) or "music". Music lowers the VAD
  // threshold so sung vocals transcribe.
  audioType: string;
  // Decoder search width: "balanced" (default) or "fast". Fast decodes greedily —
  // measured 13–23% quicker, with differences that are lateral (kana vs kanji,
  // punctuation, sentence splits) rather than less accurate. There is no "thorough"
  // option: a wider beam was measured and recovered nothing for its extra cost.
  decodeSpeed: string;
};

export type TranslationProvider = "google-translate" | "deepl";

export type TranslationSettings = {
  provider: TranslationProvider;
  // Lowercase ISO 639-1. The extension interpolates this straight into a
  // provider URL, so an uppercase or regional code (EN-US) breaks the request.
  targetLanguage: string;
};

export type ThemePreference = "system" | "light" | "dark";

// Held to scan. Must stay in lockstep with the Rust `normalize_scan_modifier`.
export type ScanModifier = "shift" | "ctrl" | "alt" | "none";
export type ScanReleaseBehavior = "remainOpen" | "close";

export type ScannerSettings = {
  modifier: ScanModifier;
  releaseBehavior: ScanReleaseBehavior;
  debounceMs: number;
  // Popup font. Empty means inherit the app's reading font.
  fontFamily: string;
  fontSizePx: number;
  // Draw our own scannable subtitles over mpv instead of mpv's styled ones.
  // Off by default: mpv's .ass rendering is what works today.
  overlayEnabled: boolean;
  overlayFontSizePx: number;
  // The app's own reading typography, driving --font-reading and --reading-base.
  readingFontFamily: string;
  readingFontSizePx: number;
};

export type JimakuEntry = {
  id: number;
  name: string | null;
  englishName: string | null;
  japaneseName: string | null;
};

export type JimakuFile = {
  name: string;
  url: string;
  size: number | null;
};

// Where the global recording-indicator toast is anchored on screen. Must stay in
// lockstep with the six values the Rust `normalize_indicator_position` accepts.
export type IndicatorPosition =
  | "top-left"
  | "top-center"
  | "top-right"
  | "bottom-left"
  | "bottom-center"
  | "bottom-right";

export type AppSettings = {
  outputDirectory: string;
  assetDirectory: string;
  whisper: WhisperSettings;
  anki: AnkiSettings;
  features: FeatureSettings;
  translation: TranslationSettings;
  scanner: ScannerSettings;
  // jimaku.cc API key. Flat because it is one field; the sibling-wiping trap that once
  // made flatness the safer choice is gone — see mergeSettings.
  jimakuApiKey: string;
  theme: ThemePreference;
  indicatorPosition: IndicatorPosition;
  launchAtLogin: boolean;
  startMinimized: boolean;
};

/**
 * A settings change: any subset of the fields, at any depth.
 *
 * Derived from AppSettings rather than written out, because the written-out version
 * existed three times and one copy had fallen two groups behind — it still omitted
 * `translation` and `scanner`, so inside `useSetupActions` those two were typed as
 * whole objects and could not be partially updated at all. Deriving it means a group
 * added to AppSettings is covered here the moment it exists.
 *
 * AppSettings holds only strings, numbers, booleans and nested groups of the same, so
 * `extends object` cleanly separates "group to recurse into" from "value to replace".
 */
// An array is a leaf, not something to recurse into. `mergeSettings` replaces an
// array wholesale — merging by index would make removing a row impossible — and
// the type has to say the same thing, or an update could offer a half-filled row
// that type-checks here and is rejected by the save as a missing field.
export type DeepPartial<T> = {
  [K in keyof T]?: T[K] extends readonly unknown[]
    ? T[K]
    : T[K] extends object
      ? DeepPartial<T[K]>
      : T[K];
};

export type SettingsUpdate = DeepPartial<AppSettings>;

// Where a video's subtitle came from. Labels a chip and nothing else — the backend keeps the
// stored value to these four, and anything else arrives as null and simply draws no chip.
export type SubtitleOrigin = "picked" | "jimaku" | "generated" | "synced";

// A video in the video library, and the subtitle it is paired with.
//
// Separate from RecentRecording on purpose: a video is watched, subtitled and realigned; a
// recording is transcribed, translated and mined. They share no actions.
export type WatchedVideo = {
  videoPath: string;
  title: string | null;
  // The remembered subtitle. Null means none has been chosen, which is not the same as none
  // existing — the container may still carry an embedded track.
  subtitlePath: string | null;
  subtitleOrigin: SubtitleOrigin | null;
  thumbnailPath: string | null;
  durationMs: number;
  bytes: number;
  addedAtMs: number;
  // Null until the video has actually been played once.
  lastOpenedAtMs: number | null;
};

export type RecentRecording = {
  fileName: string;
  filePath: string;
  // Provenance. Older state files predate these fields, so the backend
  // serializes them with #[serde(default)] and they arrive as null for every
  // recording captured before media import shipped.
  // "import" for a file brought in from disk, null/"recording" for the mic.
  source: string | null;
  sourceUrl: string | null;
  // The original file name of an imported file (the on-disk name can differ).
  title: string | null;
  transcriptPath: string | null;
  transcriptLanguage: string | null;
  transcripts: RecordingTranscript[];
  translationPath: string | null;
  ankiNoteId: number | null;
  ankiDeckName: string | null;
  ankiNoteType: string | null;
  ankiPushes: RecordingAnkiPush[];
  furiganaApplied: boolean;
  audioDeleted: boolean;
  durationMs: number;
  bytesWritten: number;
  createdAtMs: number;
};

export type RecordingSegment = {
  text: string;
  startMs: number;
  endMs: number;
};

export type RecordingTranscript = {
  language: string;
  filePath: string;
  detectedLanguage: string | null;
  segmentsPath: string | null;
};

export type RecordingAnkiPush = {
  language: string;
  deckName: string;
  noteType: string;
  noteId: number;
  furiganaApplied: boolean;
};

export type RecordingTextDocument = {
  language: string;
  detectedLanguage: string | null;
  filePath: string;
  text: string;
  missing: boolean;
  // Timed sentences parsed from the Whisper segments sidecar. Empty for older
  // recordings transcribed before timestamps were captured, and always empty
  // for translations (which have no per-sentence timing of their own).
  segments: RecordingSegment[];
};

export type RecordingTexts = {
  filePath: string;
  transcripts: RecordingTextDocument[];
  translations: RecordingTextDocument[];
};

export type WhisperDetection = {
  status: string;
  executablePath: string | null;
  modelPath: string | null;
  source: string | null;
  modelSource: string | null;
  runtimeVersion: string;
  availableRuntimeVersions: string[];
  cliReady: boolean;
  modelReady: boolean;
  cliManaged: boolean;
  modelManaged: boolean;
  message: string;
};

export type FfmpegDetection = {
  status: string;
  executablePath: string | null;
  managed: boolean;
  message: string;
};

export type YtdlpDetection = {
  status: string;
  executablePath: string | null;
  managed: boolean;
  message: string;
};

// alass is managed-only: there is no conventional system install to probe for.
export type AlassDetection = {
  status: string;
  executablePath: string | null;
  message: string;
};

export type DictionaryDetection = {
  status: string;
  dictionaryPath: string | null;
  managed: boolean;
  message: string;
};

// What the saved known-word list has to say for itself. `status` is one of
// "unconfigured" (no sources chosen), "unbuilt" (nothing saved yet), "ready",
// "stale" (the settings changed since it was built), "empty", or "offline".
export type KnownWordsSnapshot = {
  status: string;
  message: string;
  wordCount: number;
  builtAtMs: number | null;
};

export type WhisperAssetUpdateResult = {
  kind: string;
  status: string;
  message: string;
  currentVersion: string | null;
  latestVersion: string | null;
};

export type ModelDownloadSnapshot = {
  kind: string | null;
  status: string;
  message: string;
  downloadedBytes: number;
  totalBytes: number | null;
  progressPercent: number | null;
  targetPath: string | null;
};

export type AppBootstrap = {
  shell: ShellSnapshot;
  settings: AppSettings;
  recentRecordings: RecentRecording[];
  watchedVideos: WatchedVideo[];
  whisperDetection: WhisperDetection;
  ffmpegDetection: FfmpegDetection;
  ytdlpDetection: YtdlpDetection;
  alassDetection: AlassDetection;
  modelDownload: ModelDownloadSnapshot;
  dictionaryDetection: DictionaryDetection;
  knownWords: KnownWordsSnapshot;
  logPath: string;
};

export type AnkiCatalog = {
  status: string;
  message: string;
  version: number | null;
  decks: string[];
  noteTypes: string[];
  fields: string[];
};

// Sentences already mined into the configured Anki deck + note type. `status` is
// "ready" | "offline" | "unmapped"; the latter two carry an empty list and are normal
// states, not failures — a transcript still opens fine with Anki closed.
export type MinedSentences = {
  status: string;
  message: string;
  sentences: string[];
};

export type RecordingActionItem = {
  filePath: string;
  status: string;
  message: string;
  noteId: number | null;
};

export type RecordingBatchResult = {
  status: string;
  message: string;
  items: RecordingActionItem[];
  bootstrap: AppBootstrap;
};

// What one YouTube import settled as. A rejected `invoke` carries a reason but
// no `bootstrap`, so it cannot honestly be dressed up as a RecordingBatchResult
// — the reason travels on its own branch, and the queue row renders it. Note a
// user Cancel is NOT this: that resolves `ok` with a "cancelled" batch.
export type YoutubeImportOutcome =
  | { ok: true; result: RecordingBatchResult }
  | { ok: false; message: string };

// One row in the Home "From YouTube" queue. The backend import stays single-URL;
// this is the shape of a frontend-only sequential queue built on top of it.
export type YoutubeQueueItem = {
  id: string;
  url: string;
  title?: string;
  status: "queued" | "active" | "done" | "failed" | "cancelled";
  message?: string;
};

// One row in the Library transcription queue. The backend transcribe command is
// single-file; this is a frontend-only sequential queue built on top of it, so
// transcription runs NON-blocking (like the YouTube import queue) instead of the
// old full-screen busy overlay.
export type TranscriptionQueueItem = {
  id: string;
  filePath: string;
  title?: string;
  status: "queued" | "active" | "done" | "failed" | "cancelled";
  message?: string;
};

// One sentence streamed from whisper while a transcription is still running. The
// bounds are already on the recording's absolute timeline (whisper maps its VAD speech
// regions back before printing), so a live row never has to be revised once the run
// finishes — it is the same segment the sidecar ends up holding.
export type TranscriptionLiveSegment = {
  filePath: string;
  startMs: number;
  endMs: number;
  text: string;
};

// What mpv is showing right now, read over its JSON IPC channel. Every field is
// optional because mpv answers null for a property with no current value — nothing
// loaded, or no subtitle on screen — and that is a normal state, not a failure.
//
// `subtitleText` / `subtitleStartMs` / `subtitleEndMs` are the line on screen and its
// exact bounds, straight from mpv. They are what mining reads: no parsing, no sync, no
// guessing which cue the user meant.
export type WatchSnapshot = {
  connected: boolean;
  path: string | null;
  title: string | null;
  positionMs: number | null;
  durationMs: number | null;
  paused: boolean;
  subtitleText: string | null;
  subtitleStartMs: number | null;
  subtitleEndMs: number | null;
  subtitleDelayMs: number;
};

export type LookupFrequency = {
  dictionary: string;
  displayValue: string | null;
};

export type LookupPitch = {
  /// Mora index where the pitch drops. 0 is 平板 (no drop).
  position: number;
};

export type LookupEntry = {
  expression: string;
  reading: string;
  dictionary: string;
  definitions: string[];
  /// Why a conjugated form matched its dictionary form, e.g. ["past"] for 食べた.
  inflectionReasons: string[];
  frequencies: LookupFrequency[];
  pitchAccents: LookupPitch[];
};

export type LookupResult = {
  /// "ready" | "empty" | "unavailable". `unavailable` means Anki is closed, which is an
  /// ordinary state — the dictionary lives inside the add-on — not an error.
  status: "ready" | "empty" | "unavailable";
  message: string;
  /// The candidate that actually matched. Usually longer than the clicked character,
  /// and it is what gets highlighted in the line.
  term: string;
  entries: LookupEntry[];
};

/// The busy actions that fetch a managed binary or model.
///
/// A download button cannot ask only whether ITS OWN action is running. `busyAction` is set
/// the instant a button is clicked, while `downloadIsActive` comes from the backend's own
/// progress snapshot and only arrives once it has started reporting — so between the click
/// and that first report, every OTHER download button is still live. Starting a second one
/// there overwrites `busyAction`, and the first download's `finally` then clears the busy
/// state while the second is still running.
///
/// Naming the group in one place means a new download joins the guard by being added here,
/// rather than by every button remembering to name it.
export const DOWNLOAD_BUSY_ACTIONS = [
  "downloadModel",
  "downloadRuntime",
  "downloadFfmpeg",
  "downloadAlass",
  "downloadYtdlp",
  "downloadDictionary",
] as const;

export function isDownloadBusy(busyAction: BusyAction): boolean {
  return DOWNLOAD_BUSY_ACTIONS.some((action) => action === busyAction);
}

export type BusyAction =
  | "start"
  | "stop"
  | "hide"
  | "browse"
  | "downloadModel"
  | "downloadRuntime"
  | "downloadFfmpeg"
  | "downloadAlass"
  | "downloadYtdlp"
  | "downloadDictionary"
  | "refreshKnownWords"
  | "importYoutube"
  | "checkYtdlpUpdate"
  | "checkRuntimeUpdate"
  | "checkModelUpdate"
  | "loadAnki"
  | "playRecording"
  | "deleteRecording"
  | "pushAnki"
  | "mineSegment"
  | "addFurigana"
  | "translateRecording"
  | "convertMp3"
  | "importMedia"
  | null;

export type AutosaveState = "idle" | "saving" | "error";

export type AppPage =
  | "home"
  | "recordings"
  | "watch"
  | "transcript"
  | "setup"
  | "settings";

// The stacked sections inside the single Settings page. Setup-checklist rows and
// post-download navigation deep-link to one of these, scrolling it into view.
export type SettingsSection =
  | "preferences"
  | "whisper"
  | "storage"
  | "anki"
  | "studyPicks"
  | "scanner";

export type RecordingFilter =
  | "all"
  | "needsTranscription"
  | "needsAnki"
  | "needsTranslation"
  | "complete";

export type SelectOption = {
  value: string;
  label: string;
};

export type LanguageOption = {
  code: string;
  label: string;
};
