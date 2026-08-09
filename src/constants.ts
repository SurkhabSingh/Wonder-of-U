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

// The scan gesture. "None" is offered because some users genuinely want bare hover, but
// it is not the default: over a subtitle list you are reading, a popup on every pointer
// move is noise.
export const SCAN_MODIFIER_OPTIONS = [
  { value: "shift", label: "Hold Shift (like Yomitan)" },
  { value: "ctrl", label: "Hold Ctrl" },
  { value: "alt", label: "Hold Alt" },
  { value: "none", label: "No key — hover alone" },
] as const;

export const SCAN_RELEASE_OPTIONS = [
  { value: "remainOpen", label: "Leave the popup open" },
  { value: "close", label: "Close the popup" },
] as const;

export const SCAN_DEBOUNCE_OPTIONS = [
  { value: "0", label: "None" },
  { value: "20", label: "20 ms (default)" },
  { value: "60", label: "60 ms" },
  { value: "120", label: "120 ms" },
  { value: "250", label: "250 ms" },
] as const;

// Families that ship with Windows and cover Japanese, plus the app's own stack. A free
// text field would let a user name anything, but every miss renders as fallback with no
// explanation, so the list is the honest surface.
export const FONT_FAMILY_OPTIONS = [
  { value: "", label: "Match the app" },
  { value: "Yu Gothic UI", label: "Yu Gothic UI" },
  { value: "Meiryo", label: "Meiryo" },
  { value: "MS Gothic", label: "MS Gothic" },
  { value: "Noto Sans JP", label: "Noto Sans JP" },
  { value: "Segoe UI", label: "Segoe UI" },
  { value: "Georgia", label: "Georgia (serif)" },
] as const;

export const POPUP_FONT_SIZE_OPTIONS = [
  { value: "12", label: "12 px" },
  { value: "14", label: "14 px (default)" },
  { value: "16", label: "16 px" },
  { value: "18", label: "18 px" },
  { value: "22", label: "22 px" },
] as const;

export const SUBTITLE_FONT_SIZE_OPTIONS = [
  { value: "20", label: "20 px" },
  { value: "28", label: "28 px (default)" },
  { value: "36", label: "36 px" },
  { value: "44", label: "44 px" },
  { value: "56", label: "56 px" },
] as const;

export const READING_FONT_SIZE_OPTIONS = [
  { value: "15", label: "15 px" },
  { value: "17", label: "17 px (default)" },
  { value: "19", label: "19 px" },
  { value: "21", label: "21 px" },
  { value: "24", label: "24 px" },
] as const;

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
      mine: "Ctrl+Alt+M",
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
      cpuUsage: "balanced",
      audioType: "speech",
      decodeSpeed: "balanced",
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
        sourceUrl: "",
        title: "",
        position: "",
        image: "",
        video: "",
        definition: "",
        word: "",
      },
      clipPaddingMs: 250,
      vocabularySources: [],
      knownWordIntervalDays: 21,
      definitionDictionaryIds: [],
    },
    features: {
      transcription: true,
      deleteLocalAudioAfterAnkiPush: false,
      allowMp3Conversion: false,
      autoAddFuriganaAfterAnkiPush: false,
      translateAfterTranscription: false,
      addDefinitionsToMinedCards: false,
      allowDuplicateMinedWords: false,
    },
    translation: {
      provider: "google-translate",
      targetLanguage: DEFAULT_TRANSLATION_TARGET_LANGUAGE,
    },
    scanner: {
      modifier: "shift",
      releaseBehavior: "remainOpen",
      debounceMs: 20,
      fontFamily: "",
      fontSizePx: 14,
      overlayEnabled: false,
      overlayFontSizePx: 28,
      readingFontFamily: "",
      readingFontSizePx: 17,
    },
    jimakuApiKey: "",
    theme: "system",
    indicatorPosition: "top-center",
    launchAtLogin: false,
    startMinimized: false,
  },
  recentRecordings: [],
  watchedVideos: [],
  whisperDetection: {
    status: "notFound",
    executablePath: null,
    modelPath: null,
    source: null,
    modelSource: null,
    runtimeVersion: RECOMMENDED_RUNTIME_VERSION,
    availableRuntimeVersions: [],
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
  alassDetection: {
    status: "notFound",
    executablePath: null,
    message:
      "alass is not installed. Download it to align out-of-sync subtitles automatically.",
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
  dictionaryDetection: {
    status: "notFound",
    dictionaryPath: null,
    managed: false,
    message:
      "Install the Japanese dictionary to analyse transcript sentences word by word.",
  },
  knownWords: {
    status: "unconfigured",
    message: "Add a vocabulary note type and field to build the list.",
    wordCount: 0,
    builtAtMs: null,
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
