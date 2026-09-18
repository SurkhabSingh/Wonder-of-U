export type RecorderPhase =
  | "idle"
  | "recording"
  | "saving"
  | "transcribing"
  | "downloading-model"
  | "error"
  | string;

export type HotkeyBindings = {
  start: string;
  stop: string;
  showWindow: string;
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
  addDefinitionsToMinedCards: boolean;
  allowDuplicateMinedWords: boolean;
  mineWordsWithoutContext: boolean;
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
  image: string;
  video: string;
  definition: string;
  word: string;
};

export type AnkiSettings = {
  deckName: string;
  noteType: string;
  fields: AnkiFieldMapping;
  clipPaddingMs: number;
  vocabularySources: VocabularySource[];
  knownWordIntervalDays: number;
  definitionDictionaryIds: number[];
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
  cpuUsage: string;
  audioType: string;
  decodeSpeed: string;
};

export type TranslationProvider = "google-translate" | "deepl";

export type TranslationSettings = {
  provider: TranslationProvider;
  targetLanguage: string;
};

export type ThemePreference = "system" | "light" | "dark";

export type ScanModifier = "shift" | "ctrl" | "alt" | "none";
export type ScanReleaseBehavior = "remainOpen" | "close";

export type ScannerSettings = {
  modifier: ScanModifier;
  releaseBehavior: ScanReleaseBehavior;
  debounceMs: number;
  fontFamily: string;
  fontSizePx: number;
  overlayEnabled: boolean;
  overlayFontSizePx: number;
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
  jimakuApiKey: string;
  theme: ThemePreference;
  indicatorPosition: IndicatorPosition;
  launchAtLogin: boolean;
  startMinimized: boolean;
};

export type DeepPartial<T> = {
  [K in keyof T]?: T[K] extends readonly unknown[]
    ? T[K]
    : T[K] extends object
      ? DeepPartial<T[K]>
      : T[K];
};

export type SettingsUpdate = DeepPartial<AppSettings>;

export type SubtitleOrigin = "picked" | "jimaku" | "generated" | "synced";

export type WatchedVideo = {
  videoPath: string;
  title: string | null;
  subtitlePath: string | null;
  subtitleOrigin: SubtitleOrigin | null;
  thumbnailPath: string | null;
  durationMs: number;
  bytes: number;
  addedAtMs: number;
  lastOpenedAtMs: number | null;
  resumePositionMs: number | null;
};

export type RecentRecording = {
  fileName: string;
  filePath: string;
  source: string | null;
  sourceUrl: string | null;
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
  vadReady: boolean;
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

export type MpvDetection = {
  status: string;
  executablePath: string | null;
  managed: boolean;
  message: string;
};

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

export type LookupDictionary = {
  id: number;
  title: string;
  revision: string;
  enabled: boolean;
  priority: number;
  termCount: number;
};

// `status` is "ready" or "unavailable" — Anki being closed is ordinary here.
export type LookupDictionaries = {
  status: string;
  message: string;
  dictionaries: LookupDictionary[];
};

// One line handed to a batch mine.
export type MineLineRequest = {
  text: string;
  startMs: number;
  endMs: number;
  translation: string | null;
};

// What became of one line in a batch. `status` is "added", "failed", or
// "notAttempted" — the last for lines a stopped run never reached, which is not
// the same as a line that was tried and refused.
export type MinedLineOutcome = {
  text: string;
  startMs: number;
  endMs: number;
  status: string;
  message: string;
};

// `status` is "ready" (all added), "partial" (some failed), "stopped" (the run
// gave up early), or "failed" (nothing was attempted).
export type MinedLinesResult = {
  status: string;
  message: string;
  added: number;
  failed: number;
  lines: MinedLineOutcome[];
  bootstrap: AppBootstrap;
};

export type LineRanking = {
  unknownWords: string[];
  contentWordCount: number;
  withinReach: boolean;
  hasContext: boolean;
};

export type TranscriptRanking = {
  status: string;
  message: string;
  lines: LineRanking[];
};

export type VocabularySuggestion = {
  noteType: string;
  field: string;
  matureNoteCount: number;
  samples: string[];
  alreadyAdded: boolean;
};

export type VocabularySuggestions = {
  status: string;
  message: string;
  suggestions: VocabularySuggestion[];
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
  kind: AssetKind;
  status: string;
  message: string;
  currentVersion: string | null;
  latestVersion: string | null;
};

// Which asset a download is for. These are the exact strings Rust's `AssetKind` serializes
// to, and the Rust side has a test pinning them, so this list is a copy of an authoritative
// one rather than a second opinion.
export type AssetKind =
  | "model"
  | "runtime"
  | "ffmpeg"
  | "ytdlp"
  | "alass"
  | "dictionary"
  | "mpv";

export type ModelDownloadSnapshot = {
  kind: AssetKind | null;
  status: string;
  message: string;
  downloadedBytes: number;
  totalBytes: number | null;
  progressPercent: number | null;
  targetPath: string | null;
  queuedRemaining: number;
};

// One thing transcription cannot run without. Built by the backend's
// `transcription_requirements`, which is the only place that decides what the list contains —
// the Setup checklist reads it instead of deciding for itself, which is how FFmpeg came to be
// listed as optional while nothing could be transcribed without it.
export type TranscriptionRequirement = {
  // "whisper" | "ffmpeg" | "vad"
  id: string;
  ready: boolean;
};

/**
 * Whether anything transcription needs is still missing.
 */
export function transcriptionSetupIncomplete(
  requirements: TranscriptionRequirement[],
): boolean {
  return requirements.some((entry) => !entry.ready);
}

export function transcriptionReady(
  requirements: TranscriptionRequirement[],
): boolean {
  return requirements.length > 0 && requirements.every((entry) => entry.ready);
}

export type AppBootstrap = {
  shell: ShellSnapshot;
  settings: AppSettings;
  recentRecordings: RecentRecording[];
  watchedVideos: WatchedVideo[];
  whisperDetection: WhisperDetection;
  ffmpegDetection: FfmpegDetection;
  ytdlpDetection: YtdlpDetection;
  alassDetection: AlassDetection;
  mpvDetection: MpvDetection;
  modelDownload: ModelDownloadSnapshot;
  dictionaryDetection: DictionaryDetection;
  knownWords: KnownWordsSnapshot;
  transcriptionRequirements: TranscriptionRequirement[];
  logPath: string;
  loggingFailure: string | null;
};

export type AnkiCatalog = {
  status: string;
  message: string;
  version: number | null;
  decks: string[];
  noteTypes: string[];
  noteType: string;
  fields: string[] | null;
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

// What one YouTube import settled as.
export type YoutubeImportOutcome =
  | { ok: true; result: RecordingBatchResult }
  | { ok: false; message: string };

// One row in the Home "From YouTube" queue. The backend import stays single-URL;
// this is the shape of a frontend-only sequential queue built on top of it.
export type YoutubeQueueItem = {
  id: string;
  url: string;
  title?: string;
  status: "queued" | "active" | "done" | "partial" | "failed" | "cancelled";
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
  position: number;
};

export type LookupEntry = {
  expression: string;
  reading: string;
  dictionary: string;
  definitions: string[];
  inflectionReasons: string[];
  frequencies: LookupFrequency[];
  pitchAccents: LookupPitch[];
};

export type LookupResult = {
  status: "ready" | "empty" | "unavailable";
  message: string;
  term: string;
  entries: LookupEntry[];
};

/// The busy actions that fetch a managed binary or model.
export const DOWNLOAD_BUSY_ACTIONS = [
  "downloadModel",
  "downloadRuntime",
  "downloadFfmpeg",
  "downloadAlass",
  "downloadYtdlp",
  "downloadDictionary",
  "downloadMpv",
  "reinstallMpv",
  "downloadEssentials",
  "reinstallFfmpeg",
] as const;

export function isDownloadBusy(busyAction: BusyAction): boolean {
  return DOWNLOAD_BUSY_ACTIONS.some((action) => action === busyAction);
}

export type BusyAction =
  | "start"
  | "stop"
  | "hide"
  | "browse"
  | (typeof DOWNLOAD_BUSY_ACTIONS)[number]
  | "refreshKnownWords"
  | "scanVocabulary"
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
  | "progress"
  | "setup"
  | "settings";

// A number the backend worked out, together with whether it could work it out at all.
export type Measured<T> = {
  value: T | null;
  status: string;
  asOfMs: number | null;
  reason: string | null;
};

// A like-for-like change between two readings, over the material they both measured.
export type ProgressComparison = {
  deltaPoints: number;
  earlierPercent: number;
  laterPercent: number;
  itemsCompared: number;
  itemsAdded: number;
  itemsChanged: number;
  earlierTakenAtMs: number;
  laterTakenAtMs: number;
};

export type LibraryReport = {
  items: number;
  totalMs: number;
  itemsWithoutLength: number;
  recorded: number;
  importedFromALink: number;
  importedFromAFile: number;
  unknownOrigin: number;
  transcribed: number;
  japanese: number;
  translated: number;
};

export type ImmersionStreak = {
  current: number;
  longest: number;
  activeDays30: number;
  todayCounted: boolean;
};

export type ImmersionReport = {
  todayMs: number;
  weekMs: number;
  todayUnmeasuredMs: number;
  countedFrom: string;
  streak: ImmersionStreak;
};

export type CardCounts = {
  word: number;
  line: number;
  transcript: number;
  unsorted: number;
};

export type MaterialCounts = {
  recordings: number;
  videos: number;
};

// Every count here is null on a day its source did not speak for. There is no number, so
// there is no colour step to pick: never write `?? 0` here.
export type CalendarDay = {
  day: string;
  combinedMs: number | null;
  cards: CardCounts | null;
  material: MaterialCounts | null;
};

// When Anki last answered. Days after `on` have not been counted.
export type CardsCounted = {
  from: string | null;
  on: string;
  atMs: number;
  undated: number;
};

export type CalendarSpan = {
  firstDay: string | null;
  // Null when the store could not be read: nothing was counted, today included.
  countedFrom: string | null;
  materialFrom: string | null;
  // Null until Anki has answered once.
  cards: CardsCounted | null;
  days: CalendarDay[];
  droppedEvidence: number;
};

export type ReadingPoint = {
  atMs: number;
  // Points gained since the first reading, on transcripts compared unchanged.
  gained: number;
  step: number | null;
  compared: number;
  coverage: number;
  notCompared: "settingsChanged" | "tooLittleInCommon" | null;
};

export type WordsPoint = {
  atMs: number;
  words: number;
  settingsChanged: boolean;
};

export type Levels = {
  reading: ReadingPoint[];
  words: WordsPoint[];
};

export type WriteFailure = {
  sinceMs: number;
  reason: string;
};

export type ProgressReport = {
  coveragePercent: Measured<number>;
  immersion: Measured<ImmersionReport>;
  calendar: CalendarSpan;
  today: string;
  library: LibraryReport;
  // Null is "not yet", which the page says in words rather than drawing as zero.
  comparison: ProgressComparison | null;
  levels: Levels;
  readings: number;
  firstRunDay: string | null;
  damagedRows: number;
  newerRows: number;
  // False means every number above is a guess about a file nobody opened.
  storeReadable: boolean;
  // Set while saving is failing; what was done since then is not in the numbers above.
  writeFailure: WriteFailure | null;
};

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
