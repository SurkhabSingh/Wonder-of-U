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
};

// One note type + field the known-word index is read from. Independent of the
// push `noteType`: the note type cards are pushed into is rarely one vocabulary is
// read from.
export type VocabularySource = {
  noteType: string;
  field: string;
};

export type AnkiSettings = {
  deckName: string;
  noteType: string;
  fields: AnkiFieldMapping;
  // The note types the known-word index is read from, unioned together. A
  // learner's vocabulary routinely spans several note types (a Kaishi deck and a
  // Lapis deck, say). An empty list means the feature is off.
  vocabularySources: VocabularySource[];
};

export type WhisperSettings = {
  cliPath: string;
  modelPath: string;
  runtimeVersion: string;
  modelChoice: string;
  language: string;
};

export type TranslationProvider = "google-translate" | "deepl";

export type TranslationSettings = {
  provider: TranslationProvider;
  // Lowercase ISO 639-1. The extension interpolates this straight into a
  // provider URL, so an uppercase or regional code (EN-US) breaks the request.
  targetLanguage: string;
};

export type ThemePreference = "system" | "light" | "dark";

export type AppSettings = {
  outputDirectory: string;
  assetDirectory: string;
  whisper: WhisperSettings;
  anki: AnkiSettings;
  features: FeatureSettings;
  translation: TranslationSettings;
  theme: ThemePreference;
  launchAtLogin: boolean;
  startMinimized: boolean;
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

export type DictionaryDetection = {
  status: string;
  dictionaryPath: string | null;
  managed: boolean;
  message: string;
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
  whisperDetection: WhisperDetection;
  ffmpegDetection: FfmpegDetection;
  ytdlpDetection: YtdlpDetection;
  dictionaryDetection: DictionaryDetection;
  modelDownload: ModelDownloadSnapshot;
  logPath: string;
};

export type AnkiCatalog = {
  status: string;
  message: string;
  version: number | null;
  decks: string[];
  noteTypes: string[];
  fields: string[];
  // Field names keyed by note type, for the vocabulary rows. Each row picks its
  // own note type, so one flat list cannot serve them — a row reads its dropdown
  // from `vocabularyFieldMap[row.noteType]`. Holds an entry per distinct
  // configured note type (plus any the picker is currently asking about).
  vocabularyFieldMap: Record<string, string[]>;
};

// `wordCount` and `builtAtMs` describe the index as it stands, not what the last
// refresh read: an offline refresh keeps the previous index and reports it.
export type KnownWordsSnapshot = {
  status: "ready" | "empty" | "offline" | "unconfigured";
  message: string;
  wordCount: number;
  builtAtMs: number | null;
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

export type BusyAction =
  | "start"
  | "stop"
  | "hide"
  | "browse"
  | "downloadModel"
  | "downloadRuntime"
  | "downloadFfmpeg"
  | "downloadYtdlp"
  | "downloadDictionary"
  | "importYoutube"
  | "checkYtdlpUpdate"
  | "checkDictionaryUpdate"
  | "checkRuntimeUpdate"
  | "checkModelUpdate"
  | "loadAnki"
  | "knownWords"
  | "playRecording"
  | "deleteRecording"
  | "pushAnki"
  | "mineSegment"
  | "addFurigana"
  | "translateRecording"
  | "transcribeRecording"
  | "convertMp3"
  | "importMedia"
  | null;

export type AutosaveState = "idle" | "saving" | "error";

export type AppPage =
  | "home"
  | "recordings"
  | "transcript"
  | "setup"
  | "settings";

// The stacked sections inside the single Settings page. Setup-checklist rows and
// post-download navigation deep-link to one of these, scrolling it into view.
export type SettingsSection = "preferences" | "whisper" | "storage" | "anki";

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
