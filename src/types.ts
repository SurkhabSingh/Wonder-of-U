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

export type AnkiSettings = {
  deckName: string;
  noteType: string;
  fields: AnkiFieldMapping;
};

export type WhisperSettings = {
  cliPath: string;
  modelPath: string;
  runtimeVersion: string;
  modelChoice: string;
  language: string;
};

export type ThemePreference = "system" | "light" | "dark";

export type AppSettings = {
  outputDirectory: string;
  assetDirectory: string;
  whisper: WhisperSettings;
  anki: AnkiSettings;
  features: FeatureSettings;
  theme: ThemePreference;
  launchAtLogin: boolean;
  startMinimized: boolean;
};

export type RecentRecording = {
  fileName: string;
  filePath: string;
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

export type RecordingTranscript = {
  language: string;
  filePath: string;
  detectedLanguage: string | null;
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

export type BusyAction =
  | "start"
  | "stop"
  | "hide"
  | "browse"
  | "downloadModel"
  | "downloadRuntime"
  | "downloadFfmpeg"
  | "checkRuntimeUpdate"
  | "checkModelUpdate"
  | "loadAnki"
  | "playRecording"
  | "deleteRecording"
  | "pushAnki"
  | "addFurigana"
  | "translateRecording"
  | "transcribeRecording"
  | "convertMp3"
  | null;

export type AutosaveState = "idle" | "saving" | "error";

export type AppPage =
  | "recorder"
  | "recordings"
  | "transcript"
  | "preferences"
  | "whisper"
  | "runtime"
  | "model"
  | "storage"
  | "anki";

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
