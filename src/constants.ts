import type { AnkiCatalog, AppBootstrap, LanguageOption } from "./types";

export const MODEL_OPTIONS = [
  {
    id: "tiny",
    label: "Tiny",
    description: "Fastest option with the lightest RAM footprint.",
    diskSize: "75 MiB",
    memoryUsage: "~273 MB",
  },
  {
    id: "base",
    label: "Base",
    description: "Good entry option when you want a little more accuracy than Tiny.",
    diskSize: "142 MiB",
    memoryUsage: "~388 MB",
  },
  {
    id: "small",
    label: "Small",
    description: "Balanced multilingual default for everyday offline transcription.",
    diskSize: "466 MiB",
    memoryUsage: "~852 MB",
  },
  {
    id: "medium",
    label: "Medium",
    description: "Higher accuracy with a noticeable jump in RAM and download size.",
    diskSize: "1.5 GiB",
    memoryUsage: "~2.1 GB",
  },
  {
    id: "large-v3",
    label: "Large v3",
    description: "Best accuracy, but also the heaviest CPU, RAM, and disk option.",
    diskSize: "2.9 GiB",
    memoryUsage: "~3.9 GB",
  },
] as const;

export const RECOMMENDED_RUNTIME_VERSION = "v1.8.4";

export const LANGUAGE_OPTIONS = [
  { code: "auto", label: "Auto detect" },
  { code: "af", label: "Afrikaans" },
  { code: "am", label: "Amharic" },
  { code: "ar", label: "Arabic" },
  { code: "as", label: "Assamese" },
  { code: "az", label: "Azerbaijani" },
  { code: "ba", label: "Bashkir" },
  { code: "be", label: "Belarusian" },
  { code: "bg", label: "Bulgarian" },
  { code: "bn", label: "Bengali" },
  { code: "bo", label: "Tibetan" },
  { code: "br", label: "Breton" },
  { code: "bs", label: "Bosnian" },
  { code: "ca", label: "Catalan" },
  { code: "cs", label: "Czech" },
  { code: "cy", label: "Welsh" },
  { code: "da", label: "Danish" },
  { code: "de", label: "German" },
  { code: "el", label: "Greek" },
  { code: "en", label: "English" },
  { code: "es", label: "Spanish" },
  { code: "et", label: "Estonian" },
  { code: "eu", label: "Basque" },
  { code: "fa", label: "Persian" },
  { code: "fi", label: "Finnish" },
  { code: "fo", label: "Faroese" },
  { code: "fr", label: "French" },
  { code: "gl", label: "Galician" },
  { code: "gu", label: "Gujarati" },
  { code: "ha", label: "Hausa" },
  { code: "haw", label: "Hawaiian" },
  { code: "he", label: "Hebrew" },
  { code: "hi", label: "Hindi" },
  { code: "hr", label: "Croatian" },
  { code: "ht", label: "Haitian Creole" },
  { code: "hu", label: "Hungarian" },
  { code: "hy", label: "Armenian" },
  { code: "id", label: "Indonesian" },
  { code: "is", label: "Icelandic" },
  { code: "it", label: "Italian" },
  { code: "ja", label: "Japanese" },
  { code: "jw", label: "Javanese" },
  { code: "ka", label: "Georgian" },
  { code: "kk", label: "Kazakh" },
  { code: "km", label: "Khmer" },
  { code: "kn", label: "Kannada" },
  { code: "ko", label: "Korean" },
  { code: "la", label: "Latin" },
  { code: "lb", label: "Luxembourgish" },
  { code: "ln", label: "Lingala" },
  { code: "lo", label: "Lao" },
  { code: "lt", label: "Lithuanian" },
  { code: "lv", label: "Latvian" },
  { code: "mg", label: "Malagasy" },
  { code: "mi", label: "Maori" },
  { code: "mk", label: "Macedonian" },
  { code: "ml", label: "Malayalam" },
  { code: "mn", label: "Mongolian" },
  { code: "mr", label: "Marathi" },
  { code: "ms", label: "Malay" },
  { code: "mt", label: "Maltese" },
  { code: "my", label: "Myanmar" },
  { code: "ne", label: "Nepali" },
  { code: "nl", label: "Dutch" },
  { code: "nn", label: "Nynorsk" },
  { code: "no", label: "Norwegian" },
  { code: "oc", label: "Occitan" },
  { code: "pa", label: "Punjabi" },
  { code: "pl", label: "Polish" },
  { code: "ps", label: "Pashto" },
  { code: "pt", label: "Portuguese" },
  { code: "ro", label: "Romanian" },
  { code: "ru", label: "Russian" },
  { code: "sa", label: "Sanskrit" },
  { code: "sd", label: "Sindhi" },
  { code: "si", label: "Sinhala" },
  { code: "sk", label: "Slovak" },
  { code: "sl", label: "Slovenian" },
  { code: "sn", label: "Shona" },
  { code: "so", label: "Somali" },
  { code: "sq", label: "Albanian" },
  { code: "sr", label: "Serbian" },
  { code: "su", label: "Sundanese" },
  { code: "sv", label: "Swedish" },
  { code: "sw", label: "Swahili" },
  { code: "ta", label: "Tamil" },
  { code: "te", label: "Telugu" },
  { code: "tg", label: "Tajik" },
  { code: "th", label: "Thai" },
  { code: "tk", label: "Turkmen" },
  { code: "tl", label: "Tagalog" },
  { code: "tr", label: "Turkish" },
  { code: "tt", label: "Tatar" },
  { code: "uk", label: "Ukrainian" },
  { code: "ur", label: "Urdu" },
  { code: "uz", label: "Uzbek" },
  { code: "vi", label: "Vietnamese" },
  { code: "yi", label: "Yiddish" },
  { code: "yo", label: "Yoruba" },
  { code: "yue", label: "Cantonese" },
  { code: "zh", label: "Chinese" },
] as const;

/* Translation targets ---------------------------------------------------------
   The UI owns these menus: the backend only normalizes the format (trim +
   lowercase + fall back to "en") and deliberately does not check the code
   against any list, so an unsupported code survives all the way to the
   extension and surfaces as a confusing bridge error at translate time.

   Codes stay lowercase ISO 639-1 because the extension interpolates them into a
   provider URL verbatim (Google's ?tl=, DeepL's #<src>/<tgt>/<text> fragment).

   "auto" is dropped for both: it is a Whisper source-detection sentinel and is
   meaningless as a translation target. */
export const GOOGLE_TARGET_LANGUAGE_OPTIONS: readonly LanguageOption[] =
  LANGUAGE_OPTIONS.filter((option) => option.code !== "auto");

/* DeepL translates into roughly a third of what Google does, so it needs its own
   list rather than a filter over the Whisper one. Entries are limited to the
   long-standing DeepL API v2 target set; anything newer or beta-only is left out
   on purpose, since a wrong entry here fails at translate time rather than here.

   Bare en/pt are correct despite DeepL wanting EN-US/PT-PT — the extension holds
   that mapping itself (deepl-api-provider.js TARGET_LANGUAGE_OVERRIDES) and
   upper-cases everything else, so regional variants must NOT be added.

   Norwegian is "nb" (Bokmal), not the "no" the Whisper list uses: DeepL rejects
   NO. That is why switching providers re-checks the persisted code both ways
   instead of assuming Google's list is a superset of this one. */
export const DEEPL_TARGET_LANGUAGE_OPTIONS: readonly LanguageOption[] = [
  { code: "ar", label: "Arabic" },
  { code: "bg", label: "Bulgarian" },
  { code: "cs", label: "Czech" },
  { code: "da", label: "Danish" },
  { code: "de", label: "German" },
  { code: "el", label: "Greek" },
  { code: "en", label: "English" },
  { code: "es", label: "Spanish" },
  { code: "et", label: "Estonian" },
  { code: "fi", label: "Finnish" },
  { code: "fr", label: "French" },
  { code: "hu", label: "Hungarian" },
  { code: "id", label: "Indonesian" },
  { code: "it", label: "Italian" },
  { code: "ja", label: "Japanese" },
  { code: "ko", label: "Korean" },
  { code: "lt", label: "Lithuanian" },
  { code: "lv", label: "Latvian" },
  { code: "nb", label: "Norwegian Bokmal" },
  { code: "nl", label: "Dutch" },
  { code: "pl", label: "Polish" },
  { code: "pt", label: "Portuguese" },
  { code: "ro", label: "Romanian" },
  { code: "ru", label: "Russian" },
  { code: "sk", label: "Slovak" },
  { code: "sl", label: "Slovenian" },
  { code: "sv", label: "Swedish" },
  { code: "tr", label: "Turkish" },
  { code: "uk", label: "Ukrainian" },
  { code: "zh", label: "Chinese" },
];

export const DEFAULT_TRANSLATION_TARGET_LANGUAGE = "en";

/* Media import ---------------------------------------------------------------
   ONE extension list, shared by the file-picker filter and the drag-drop
   filter so the two can never drift apart.

   whisper.cpp reads the "native" formats directly, so the backend copies those
   into the recordings folder verbatim. The "convert" formats it cannot read, so
   the backend transcodes them to MP3 with ffmpeg — which means importing one of
   those requires ffmpeg to be installed (the backend fails that file with a
   clear message if it is not). */
export const IMPORT_NATIVE_EXTENSIONS = ["wav", "mp3", "flac", "ogg"] as const;

export const IMPORT_CONVERT_EXTENSIONS = [
  "m4a",
  "opus",
  "mp4",
  "webm",
  "aac",
  "mkv",
  "mov",
  "m4v",
  "wma",
  "aiff",
] as const;

export const IMPORT_MEDIA_EXTENSIONS: readonly string[] = [
  ...IMPORT_NATIVE_EXTENSIONS,
  ...IMPORT_CONVERT_EXTENSIONS,
];

export const APP_SNAPSHOT_EVENT = "app://snapshot-changed";
export const MP3_CONVERSION_WARNING =
  "MP3 reduces file size but uses lossy compression, so audio quality may be lower. Existing Anki cards are not affected.";

export const DEFAULT_BOOTSTRAP: AppBootstrap = {
  shell: {
    phase: "idle",
    statusText:
      "Tray shell is ready. Press Ctrl+Alt+R to start recording system audio.",
    lastShortcut: null,
    transitionCount: 0,
    hotkeys: {
      start: "Ctrl+Alt+R",
      stop: "Ctrl+Alt+S",
      showWindow: "Ctrl+Alt+W",
    },
    startedAtMs: null,
    currentRecordingName: null,
    lastOutputPath: null,
    lastTranscriptPath: null,
  },
  settings: {
    outputDirectory: "",
    assetDirectory: "",
    whisper: {
      cliPath: "",
      modelPath: "",
      runtimeVersion: RECOMMENDED_RUNTIME_VERSION,
      modelChoice: "small",
      language: "auto",
      highAccuracyTimestamps: false,
    },
    anki: {
      deckName: "",
      noteType: "",
      fields: {
        transcription: "",
        furigana: "",
        audio: "",
        translation: "",
        sourcePath: "",
        createdAt: "",
      },
    },
    features: {
      transcription: true,
      deleteLocalAudioAfterAnkiPush: false,
      allowMp3Conversion: false,
      autoAddFuriganaAfterAnkiPush: false,
      translateAfterTranscription: false,
    },
    translation: {
      provider: "google-translate",
      targetLanguage: DEFAULT_TRANSLATION_TARGET_LANGUAGE,
    },
    theme: "system",
    indicatorPosition: "top-center",
    launchAtLogin: false,
    startMinimized: false,
  },
  recentRecordings: [],
  whisperDetection: {
    status: "notFound",
    executablePath: null,
    modelPath: null,
    source: null,
    modelSource: null,
    runtimeVersion: RECOMMENDED_RUNTIME_VERSION,
    availableRuntimeVersions: [],
    vadModelReady: false,
    vadModelPath: null,
    cliReady: false,
    modelReady: false,
    cliManaged: false,
    modelManaged: false,
    message:
      "Add or download whisper-cli and a Whisper model to enable offline transcription.",
  },
  ffmpegDetection: {
    status: "notFound",
    executablePath: null,
    managed: false,
    message: "Install app-managed FFmpeg to compress transcribed WAV recordings into MP3.",
  },
  ytdlpDetection: {
    status: "notFound",
    executablePath: null,
    managed: false,
    message: "Install app-managed yt-dlp to import audio from YouTube and other sites.",
  },
  modelDownload: {
    kind: null,
    status: "idle",
    message: "No download in progress.",
    downloadedBytes: 0,
    totalBytes: null,
    progressPercent: null,
    targetPath: null,
  },
  logPath: "",
};

export const DEFAULT_ANKI_CATALOG: AnkiCatalog = {
  status: "idle",
  message: "Connect to Anki to load decks, note types, and fields.",
  version: null,
  decks: [],
  noteTypes: [],
  fields: [],
};
