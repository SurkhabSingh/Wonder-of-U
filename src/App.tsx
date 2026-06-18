import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

type RecorderPhase =
  | "idle"
  | "recording"
  | "saving"
  | "transcribing"
  | "error"
  | string;

type HotkeyBindings = {
  start: string;
  stop: string;
  showWindow: string;
};

type ShellSnapshot = {
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

type FeatureSettings = {
  transcription: boolean;
};

type AnkiFieldMapping = {
  transcription: string;
  audio: string;
  translation: string;
  sourcePath: string;
  createdAt: string;
};

type AnkiSettings = {
  deckName: string;
  noteType: string;
  fields: AnkiFieldMapping;
};

type WhisperSettings = {
  cliPath: string;
  modelPath: string;
  runtimeVersion: string;
  modelChoice: string;
  language: string;
};

type ThemePreference = "system" | "light" | "dark";

type AppSettings = {
  outputDirectory: string;
  assetDirectory: string;
  whisper: WhisperSettings;
  anki: AnkiSettings;
  features: FeatureSettings;
  theme: ThemePreference;
  launchAtLogin: boolean;
  startMinimized: boolean;
};

type RecentRecording = {
  fileName: string;
  filePath: string;
  transcriptPath: string | null;
  translationPath: string | null;
  ankiNoteId: number | null;
  ankiDeckName: string | null;
  ankiNoteType: string | null;
  durationMs: number;
  bytesWritten: number;
  createdAtMs: number;
};

type WhisperDetection = {
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

type FfmpegDetection = {
  status: string;
  executablePath: string | null;
  managed: boolean;
  message: string;
};

type WhisperAssetUpdateResult = {
  kind: string;
  status: string;
  message: string;
  currentVersion: string | null;
  latestVersion: string | null;
};

type ModelDownloadSnapshot = {
  kind: string | null;
  status: string;
  message: string;
  downloadedBytes: number;
  totalBytes: number | null;
  progressPercent: number | null;
  targetPath: string | null;
};

type AppBootstrap = {
  shell: ShellSnapshot;
  settings: AppSettings;
  recentRecordings: RecentRecording[];
  whisperDetection: WhisperDetection;
  ffmpegDetection: FfmpegDetection;
  modelDownload: ModelDownloadSnapshot;
  logPath: string;
};

type AnkiCatalog = {
  status: string;
  message: string;
  version: number | null;
  decks: string[];
  noteTypes: string[];
  fields: string[];
};

type RecordingActionItem = {
  filePath: string;
  status: string;
  message: string;
  noteId: number | null;
};

type RecordingBatchResult = {
  status: string;
  message: string;
  items: RecordingActionItem[];
  bootstrap: AppBootstrap;
};

type BusyAction =
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
  | "translateRecording"
  | "transcribeRecording"
  | null;

type AutosaveState = "idle" | "saving" | "error";
type AppWarning = {
  id: number;
  message: string;
};
type AppTab = "recorder" | "settings";
type RecordingFilter =
  | "all"
  | "needsTranscription"
  | "needsAnki"
  | "needsTranslation"
  | "complete";

const MODEL_OPTIONS = [
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

const RECOMMENDED_RUNTIME_VERSION = "v1.8.4";

const LANGUAGE_OPTIONS = [
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

const APP_SNAPSHOT_EVENT = "app://snapshot-changed";
const DEFAULT_BOOTSTRAP: AppBootstrap = {
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
    },
    anki: {
      deckName: "",
      noteType: "",
      fields: {
        transcription: "",
        audio: "",
        translation: "",
        sourcePath: "",
        createdAt: "",
      },
    },
    features: {
      transcription: true,
    },
    theme: "system",
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

const DEFAULT_ANKI_CATALOG: AnkiCatalog = {
  status: "idle",
  message: "Connect to Anki to load decks, note types, and fields.",
  version: null,
  decks: [],
  noteTypes: [],
  fields: [],
};

function formatDuration(durationMs: number): string {
  const totalSeconds = Math.max(0, Math.floor(durationMs / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;

  if (hours > 0) {
    return `${hours.toString().padStart(2, "0")}:${minutes
      .toString()
      .padStart(2, "0")}:${seconds.toString().padStart(2, "0")}`;
  }

  return `${minutes.toString().padStart(2, "0")}:${seconds
    .toString()
    .padStart(2, "0")}`;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`;
  }

  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`;
  }

  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function formatProgressBytes(
  downloadedBytes: number,
  totalBytes: number | null,
): string {
  if (totalBytes && totalBytes > 0) {
    return `${formatBytes(downloadedBytes)} / ${formatBytes(totalBytes)}`;
  }

  return formatBytes(downloadedBytes);
}

function formatTimestamp(timestampMs: number): string {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(timestampMs);
}

function fileNameFromPath(path: string | null): string {
  if (!path) {
    return "";
  }

  const segments = path.split(/[\\/]/).filter(Boolean);
  return segments[segments.length - 1] ?? path;
}

function normalizeSelection(
  selection: string | string[] | null,
): string | null {
  if (!selection) {
    return null;
  }

  return Array.isArray(selection) ? selection[0] ?? null : selection;
}

function whisperStatusLabel(status: string): string {
  switch (status) {
    case "ready":
      return "Ready";
    case "cliMissing":
      return "CLI Missing";
    case "modelMissing":
      return "Model Missing";
    case "invalid":
      return "Invalid";
    default:
      return "Needs Setup";
  }
}

function TooltipBadge({
  label,
  description,
}: {
  label: string;
  description: string;
}) {
  return (
    <span className="tooltip-badge" title={description} aria-label={description}>
      {label}
    </span>
  );
}

function App() {
  const [bootstrap, setBootstrap] = useState<AppBootstrap>(DEFAULT_BOOTSTRAP);
  const [settingsDraft, setSettingsDraft] = useState<AppSettings>(
    DEFAULT_BOOTSTRAP.settings,
  );
  const [systemTheme, setSystemTheme] = useState<"light" | "dark">(() =>
    window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light",
  );
  const [busyAction, setBusyAction] = useState<BusyAction>(null);
  const [autosaveState, setAutosaveState] = useState<AutosaveState>("idle");
  const [autosaveMessage, setAutosaveMessage] = useState(
    "Changes save automatically.",
  );
  const [loadError, setLoadError] = useState("");
  const [appWarning, setAppWarning] = useState<AppWarning | null>(null);
  const [clockMs, setClockMs] = useState(() => Date.now());
  const [activeTab, setActiveTab] = useState<AppTab>("recorder");
  const [runtimeUpdateResult, setRuntimeUpdateResult] =
    useState<WhisperAssetUpdateResult | null>(null);
  const [modelUpdateResult, setModelUpdateResult] =
    useState<WhisperAssetUpdateResult | null>(null);
  const [ankiCatalog, setAnkiCatalog] =
    useState<AnkiCatalog>(DEFAULT_ANKI_CATALOG);
  const [recordingActionMessage, setRecordingActionMessage] = useState("");
  const [selectedRecordings, setSelectedRecordings] = useState<string[]>([]);
  const [recordingFilter, setRecordingFilter] = useState<RecordingFilter>("all");
  const settingsDirtyRef = useRef(false);
  const currentDraftKeyRef = useRef("");

  const settingsDraftKey = useMemo(
    () => JSON.stringify(settingsDraft),
    [settingsDraft],
  );
  const savedSettingsKey = useMemo(
    () => JSON.stringify(bootstrap.settings),
    [bootstrap.settings],
  );
  const settingsDirty = settingsDraftKey !== savedSettingsKey;
  const resolvedTheme =
    settingsDraft.theme === "system" ? systemTheme : settingsDraft.theme;

  useEffect(() => {
    settingsDirtyRef.current = settingsDirty;
    currentDraftKeyRef.current = settingsDraftKey;
  }, [settingsDirty, settingsDraftKey]);

  useEffect(() => {
    if (!appWarning) {
      return;
    }

    const timer = window.setTimeout(() => {
      setAppWarning((current) =>
        current?.id === appWarning.id ? null : current,
      );
    }, 5000);

    return () => window.clearTimeout(timer);
  }, [appWarning]);

  useEffect(() => {
    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    const updateSystemTheme = (event: MediaQueryListEvent | MediaQueryList) => {
      setSystemTheme(event.matches ? "dark" : "light");
    };

    updateSystemTheme(mediaQuery);
    mediaQuery.addEventListener("change", updateSystemTheme);

    return () => {
      mediaQuery.removeEventListener("change", updateSystemTheme);
    };
  }, []);

  useEffect(() => {
    document.documentElement.dataset.theme = resolvedTheme;
    document.documentElement.style.colorScheme = resolvedTheme;
  }, [resolvedTheme]);

  function applyBootstrap(
    nextBootstrap: AppBootstrap,
    options?: { preserveDraft?: boolean },
  ) {
    setBootstrap(nextBootstrap);
    if (!options?.preserveDraft) {
      setSettingsDraft(nextBootstrap.settings);
    }
    setLoadError("");
  }

  function showWarning(message: string) {
    setAppWarning({ id: Date.now(), message });
  }

  useEffect(() => {
    let mounted = true;

    async function loadBootstrap() {
      try {
        const nextBootstrap = await invoke<AppBootstrap>("get_app_bootstrap");
        if (!mounted) {
          return;
        }

        applyBootstrap(nextBootstrap);
        setAutosaveState("idle");
        setAutosaveMessage("Changes save automatically.");
      } catch (error) {
        if (!mounted) {
          return;
        }

        setLoadError(
          error instanceof Error
            ? error.message
            : "The Wonder of U desktop state could not be loaded.",
        );
      }
    }

    void loadBootstrap();

    const unlistenPromise = listen<AppBootstrap>(APP_SNAPSHOT_EVENT, (event) => {
      setBootstrap(event.payload);
      if (!settingsDirtyRef.current) {
        setSettingsDraft(event.payload.settings);
      }
    });

    return () => {
      mounted = false;
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    if (
      bootstrap.shell.startedAtMs === null ||
      (bootstrap.shell.phase !== "recording" &&
        bootstrap.shell.phase !== "saving")
    ) {
      setClockMs(Date.now());
      return;
    }

    setClockMs(Date.now());
    const timer = window.setInterval(() => {
      setClockMs(Date.now());
    }, 1000);

    return () => {
      window.clearInterval(timer);
    };
  }, [bootstrap.shell.phase, bootstrap.shell.startedAtMs]);

  useEffect(() => {
    setSelectedRecordings((current) =>
      current.filter((filePath) =>
        bootstrap.recentRecordings.some(
          (recording) => recording.filePath === filePath,
        ),
      ),
    );
  }, [bootstrap.recentRecordings]);

  useEffect(() => {
    if (!settingsDirty) {
      if (autosaveState !== "error") {
        setAutosaveState("idle");
        setAutosaveMessage("Changes save automatically.");
      }
      return;
    }

    const draftKeyAtSchedule = settingsDraftKey;
    const timer = window.setTimeout(async () => {
      try {
        setAutosaveState("saving");
        setAutosaveMessage("Saving changes...");
        const nextBootstrap = await invoke<AppBootstrap>("save_settings", {
          settings: settingsDraft,
        });
        const preserveDraft = currentDraftKeyRef.current !== draftKeyAtSchedule;
        applyBootstrap(nextBootstrap, { preserveDraft });
        if (!preserveDraft) {
          setAutosaveState("idle");
          setAutosaveMessage("All changes saved.");
        }
      } catch (error) {
        setAutosaveState("error");
        setAutosaveMessage(
          error instanceof Error
            ? error.message
            : "The updated settings could not be saved.",
        );
      }
    }, 320);

    return () => {
      window.clearTimeout(timer);
    };
  }, [settingsDraft, settingsDraftKey, settingsDirty]);

  useEffect(() => {
    setRuntimeUpdateResult(null);
  }, [
    settingsDraft.assetDirectory,
    settingsDraft.whisper.cliPath,
    settingsDraft.whisper.runtimeVersion,
  ]);

  useEffect(() => {
    setModelUpdateResult(null);
  }, [
    settingsDraft.assetDirectory,
    settingsDraft.whisper.modelChoice,
    settingsDraft.whisper.modelPath,
  ]);

  const elapsedRecordingMs =
    bootstrap.shell.startedAtMs !== null &&
    (bootstrap.shell.phase === "recording" || bootstrap.shell.phase === "saving")
      ? Math.max(0, clockMs - bootstrap.shell.startedAtMs)
      : 0;

  const isRecording = bootstrap.shell.phase === "recording";
  const isSaving = bootstrap.shell.phase === "saving";
  const isTranscribing = bootstrap.shell.phase === "transcribing";
  const recorderBusy =
    isRecording ||
    isSaving ||
    isTranscribing ||
    busyAction === "start" ||
    busyAction === "stop" ||
    busyAction === "transcribeRecording";
  const showBusyOverlay = isSaving || isTranscribing;
  const busyOverlayLabel = isTranscribing
    ? "Transcribing the saved recording..."
    : isSaving
      ? "Finalizing the recording..."
      : "";
  const downloadIsActive =
    bootstrap.modelDownload.status === "starting" ||
    bootstrap.modelDownload.status === "downloading" ||
    bootstrap.modelDownload.status === "paused" ||
    bootstrap.modelDownload.status === "cancelling";
  const hotkeyTooltip = `Start recording: ${bootstrap.shell.hotkeys.start}\nStop recording: ${bootstrap.shell.hotkeys.stop}\nShow window: ${bootstrap.shell.hotkeys.showWindow}`;
  const selectedModel =
    MODEL_OPTIONS.find((option) => option.id === settingsDraft.whisper.modelChoice) ??
    MODEL_OPTIONS[2];
  const activeRuntimeVersion =
    settingsDraft.whisper.runtimeVersion ||
    bootstrap.whisperDetection.runtimeVersion ||
    RECOMMENDED_RUNTIME_VERSION;
  const installedRuntimeVersions = Array.from(
    new Set([
      ...bootstrap.whisperDetection.availableRuntimeVersions,
      ...(bootstrap.whisperDetection.cliManaged ? [activeRuntimeVersion] : []),
    ]),
  ).sort();
  const manualRuntimeOverride = settingsDraft.whisper.cliPath.trim().length > 0;
  const runtimeUpdateVersion =
    runtimeUpdateResult?.status === "available"
      ? runtimeUpdateResult.latestVersion
      : null;
  const selectedLanguageCode = settingsDraft.whisper.language || "auto";
  const selectedLanguageKnown = LANGUAGE_OPTIONS.some(
    (option) => option.code === selectedLanguageCode,
  );
  const runtimeInstalled = bootstrap.whisperDetection.cliReady;
  const modelInstalled = bootstrap.whisperDetection.modelReady;
  const resolvedCliPath =
    settingsDraft.whisper.cliPath ||
    (bootstrap.whisperDetection.cliManaged
      ? bootstrap.whisperDetection.executablePath ?? ""
      : "");
  const resolvedModelPath =
    settingsDraft.whisper.modelPath ||
    (bootstrap.whisperDetection.modelManaged
      ? bootstrap.whisperDetection.modelPath ?? ""
      : "");
  const selectedRecordingSet = new Set(selectedRecordings);
  const transcribedRecordings = bootstrap.recentRecordings.filter(
    (recording) => recording.transcriptPath,
  );
  const untranscribedRecordings = bootstrap.recentRecordings.filter(
    (recording) => !recording.transcriptPath,
  );
  const recordingPushedToCurrentAnkiTarget = (recording: RecentRecording) =>
    recording.ankiNoteId !== null &&
    recording.ankiDeckName === settingsDraft.anki.deckName &&
    recording.ankiNoteType === settingsDraft.anki.noteType;
  const pushableRecordings = transcribedRecordings.filter(
    (recording) => !recordingPushedToCurrentAnkiTarget(recording),
  );
  const untranslatedRecordings = transcribedRecordings.filter(
    (recording) => recording.translationPath === null,
  );
  const completeRecordings = bootstrap.recentRecordings.filter(
    (recording) =>
      Boolean(recording.transcriptPath) &&
      recordingPushedToCurrentAnkiTarget(recording) &&
      recording.translationPath !== null,
  );
  const visibleRecordings =
    recordingFilter === "needsTranscription"
      ? untranscribedRecordings
      : recordingFilter === "needsAnki"
        ? pushableRecordings
        : recordingFilter === "needsTranslation"
          ? untranslatedRecordings
          : recordingFilter === "complete"
            ? completeRecordings
            : bootstrap.recentRecordings;
  const visibleSelectedRecordings = visibleRecordings.filter((recording) =>
    selectedRecordingSet.has(recording.filePath),
  );
  const visibleSelectedPaths = visibleSelectedRecordings.map(
    (recording) => recording.filePath,
  );
  const selectedTranscribedRecordings = visibleSelectedRecordings.filter(
    (recording) => recording.transcriptPath,
  );
  const selectedPushableRecordings = selectedTranscribedRecordings.filter(
    (recording) => !recordingPushedToCurrentAnkiTarget(recording),
  );
  const selectedUntranscribedRecordings = visibleSelectedRecordings.filter(
    (recording) => !recording.transcriptPath,
  );
  const selectedUntranslatedRecordings = selectedTranscribedRecordings.filter(
    (recording) => recording.translationPath === null,
  );
  const recordingFilterTabs: Array<{
    id: RecordingFilter;
    label: string;
    count: number;
  }> = [
    { id: "all", label: "All", count: bootstrap.recentRecordings.length },
    {
      id: "needsTranscription",
      label: "Needs transcript",
      count: untranscribedRecordings.length,
    },
    { id: "needsAnki", label: "Needs Anki", count: pushableRecordings.length },
    {
      id: "needsTranslation",
      label: "Needs translation",
      count: untranslatedRecordings.length,
    },
    { id: "complete", label: "Complete", count: completeRecordings.length },
  ];

  function updateSettings(
    update: Partial<Omit<AppSettings, "features" | "whisper" | "anki">> & {
      features?: Partial<FeatureSettings>;
      whisper?: Partial<WhisperSettings>;
      anki?: Partial<Omit<AnkiSettings, "fields">> & {
        fields?: Partial<AnkiFieldMapping>;
      };
    },
  ) {
    setSettingsDraft((current) => {
      const nextFeatures: FeatureSettings = {
        ...current.features,
        ...(update.features ?? {}),
      };
      const nextWhisper: WhisperSettings = {
        ...current.whisper,
        ...(update.whisper ?? {}),
      };
      const nextAnki: AnkiSettings = {
        ...current.anki,
        ...(update.anki ?? {}),
        fields: {
          ...current.anki.fields,
          ...(update.anki?.fields ?? {}),
        },
      };

      return {
        ...current,
        ...update,
        whisper: nextWhisper,
        anki: nextAnki,
        features: nextFeatures,
      };
    });
  }

  async function persistSettingsIfNeeded() {
    if (!settingsDirty) {
      return;
    }

    try {
      const draftKeyAtSave = currentDraftKeyRef.current;
      setAutosaveState("saving");
      setAutosaveMessage("Saving changes...");
      const nextBootstrap = await invoke<AppBootstrap>("save_settings", {
        settings: settingsDraft,
      });
      const preserveDraft = currentDraftKeyRef.current !== draftKeyAtSave;
      applyBootstrap(nextBootstrap, { preserveDraft });
      if (!preserveDraft) {
        setAutosaveState("idle");
        setAutosaveMessage("All changes saved.");
      }
    } catch (error) {
      setAutosaveState("error");
      setAutosaveMessage(
        error instanceof Error
          ? error.message
          : "The updated settings could not be saved.",
      );
      throw error;
    }
  }

  async function startRecording() {
    try {
      setBusyAction("start");
      await persistSettingsIfNeeded();
      const nextBootstrap = await invoke<AppBootstrap>("start_recording", {
        requestedName: null,
      });
      applyBootstrap(nextBootstrap);
    } catch (error) {
      setLoadError(
        error instanceof Error
          ? error.message
          : "Recording could not be started.",
      );
    } finally {
      setBusyAction(null);
    }
  }

  async function stopRecording() {
    try {
      setBusyAction("stop");
      const nextBootstrap = await invoke<AppBootstrap>("stop_recording");
      applyBootstrap(nextBootstrap);
    } catch (error) {
      setLoadError(
        error instanceof Error
          ? error.message
          : "Recording could not be stopped.",
      );
    } finally {
      setBusyAction(null);
    }
  }

  async function hideToTray() {
    try {
      setBusyAction("hide");
      await invoke("hide_main_window");
    } catch (error) {
      setLoadError(
        error instanceof Error
          ? error.message
          : "The window could not be hidden to the tray.",
      );
    } finally {
      setBusyAction(null);
    }
  }

  async function downloadRuntimeVersion(runtimeVersion: string) {
    try {
      setBusyAction("downloadRuntime");
      setRuntimeUpdateResult(null);
      await persistSettingsIfNeeded();
      const nextBootstrap = await invoke<AppBootstrap>(
        "download_whisper_runtime_version",
        { runtimeVersion },
      );
      applyBootstrap(nextBootstrap);
      setActiveTab("settings");
    } catch (error) {
      setLoadError(
        error instanceof Error
          ? error.message
          : "The selected Whisper runtime could not be prepared.",
      );
    } finally {
      setBusyAction(null);
    }
  }

  async function downloadRecommendedRuntime() {
    await downloadRuntimeVersion(RECOMMENDED_RUNTIME_VERSION);
  }

  async function downloadRecommendedFfmpeg() {
    try {
      setBusyAction("downloadFfmpeg");
      const nextBootstrap = await invoke<AppBootstrap>(
        "download_recommended_ffmpeg",
      );
      applyBootstrap(nextBootstrap);
      setActiveTab("settings");
    } catch (error) {
      setLoadError(
        error instanceof Error ? error.message : "FFmpeg could not be prepared.",
      );
    } finally {
      setBusyAction(null);
    }
  }

  async function downloadRecommendedModel() {
    try {
      setBusyAction("downloadModel");
      setModelUpdateResult(null);
      await persistSettingsIfNeeded();
      const nextBootstrap = await invoke<AppBootstrap>(
        "download_recommended_whisper_model",
      );
      applyBootstrap(nextBootstrap);
      setActiveTab("settings");
    } catch (error) {
      setLoadError(
        error instanceof Error
          ? error.message
          : "The recommended Whisper model could not be prepared.",
      );
    } finally {
      setBusyAction(null);
    }
  }

  async function checkRuntimeUpdate() {
    try {
      setBusyAction("checkRuntimeUpdate");
      await persistSettingsIfNeeded();
      const result = await invoke<WhisperAssetUpdateResult>(
        "check_whisper_runtime_update",
      );
      setRuntimeUpdateResult(result);
    } catch (error) {
      setLoadError(
        error instanceof Error
          ? error.message
          : "The runtime update check could not be completed.",
      );
    } finally {
      setBusyAction(null);
    }
  }

  async function checkModelUpdate() {
    try {
      setBusyAction("checkModelUpdate");
      await persistSettingsIfNeeded();
      const result = await invoke<WhisperAssetUpdateResult>(
        "check_whisper_model_update",
      );
      setModelUpdateResult(result);
    } catch (error) {
      setLoadError(
        error instanceof Error
          ? error.message
          : "The model update check could not be completed.",
      );
    } finally {
      setBusyAction(null);
    }
  }

  async function toggleDownloadPause() {
    try {
      const nextBootstrap = await invoke<AppBootstrap>(
        "toggle_whisper_model_download_pause",
      );
      applyBootstrap(nextBootstrap);
    } catch (error) {
      setLoadError(
        error instanceof Error
          ? error.message
          : "The active download could not be paused or resumed.",
      );
    }
  }

  async function cancelDownload() {
    try {
      const nextBootstrap = await invoke<AppBootstrap>(
        "cancel_whisper_model_download",
      );
      applyBootstrap(nextBootstrap);
    } catch (error) {
      setLoadError(
        error instanceof Error
          ? error.message
          : "The active download could not be cancelled.",
      );
    }
  }

  async function browseForDirectory(field: "outputDirectory" | "assetDirectory") {
    try {
      setBusyAction("browse");
      const selection = normalizeSelection(
        await open({
          directory: true,
          multiple: false,
          defaultPath: settingsDraft[field] || undefined,
        }),
      );

      if (!selection) {
        return;
      }

      updateSettings({ [field]: selection });
    } catch (error) {
      setLoadError(
        error instanceof Error
          ? error.message
          : "The folder chooser could not be opened.",
      );
    } finally {
      setBusyAction(null);
    }
  }

  async function browseForFile(field: "cliPath" | "modelPath") {
    try {
      setBusyAction("browse");
      const defaultPath =
        field === "cliPath" ? resolvedCliPath : resolvedModelPath;
      const selection = normalizeSelection(
        await open({
          directory: false,
          multiple: false,
          defaultPath: defaultPath || undefined,
        }),
      );

      if (!selection) {
        return;
      }

      updateSettings({ whisper: { [field]: selection } });
    } catch (error) {
      setLoadError(
        error instanceof Error
          ? error.message
          : "The file chooser could not be opened.",
      );
    } finally {
      setBusyAction(null);
    }
  }

  function toggleRecordingSelection(filePath: string) {
    setSelectedRecordings((current) =>
      current.includes(filePath)
        ? current.filter((selectedPath) => selectedPath !== filePath)
        : [...current, filePath],
    );
  }

  function clearRecordingSelection() {
    setSelectedRecordings([]);
  }

  async function refreshAnkiCatalog(noteType?: string) {
    try {
      setBusyAction("loadAnki");
      await persistSettingsIfNeeded();
      const catalog = await invoke<AnkiCatalog>("load_anki_catalog", {
        noteType: (noteType ?? settingsDraft.anki.noteType) || null,
      });
      setAnkiCatalog(catalog);
    } catch (error) {
      setLoadError(
        error instanceof Error
          ? error.message
          : "The Anki catalog could not be loaded.",
      );
    } finally {
      setBusyAction(null);
    }
  }

  async function playRecording(filePath: string) {
    try {
      setBusyAction("playRecording");
      await invoke("play_recording", { filePath });
    } catch (error) {
      setLoadError(
        error instanceof Error ? error.message : "The audio file could not be played.",
      );
    } finally {
      setBusyAction(null);
    }
  }

  async function deleteRecording(filePath: string) {
    try {
      setBusyAction("deleteRecording");
      const nextBootstrap = await invoke<AppBootstrap>("delete_recording", {
        filePath,
      });
      applyBootstrap(nextBootstrap);
      setRecordingActionMessage("Recording deleted.");
    } catch (error) {
      setLoadError(
        error instanceof Error ? error.message : "The recording could not be deleted.",
      );
    } finally {
      setBusyAction(null);
    }
  }

  async function deleteRecordings(filePaths: string[]) {
    if (filePaths.length === 0) {
      return;
    }

    const confirmed = window.confirm(
      `Delete ${filePaths.length} selected recording${
        filePaths.length === 1 ? "" : "s"
      } and their transcript files?`,
    );
    if (!confirmed) {
      return;
    }

    try {
      setBusyAction("deleteRecording");
      const result = await invoke<RecordingBatchResult>("delete_recordings", {
        filePaths,
      });
      applyBootstrap(result.bootstrap);
      setRecordingActionMessage(result.message);
    } catch (error) {
      setLoadError(
        error instanceof Error
          ? error.message
          : "The selected recordings could not be deleted.",
      );
    } finally {
      setBusyAction(null);
    }
  }

  async function pushRecordingsToAnki(filePaths: string[]) {
    try {
      setBusyAction("pushAnki");
      await persistSettingsIfNeeded();
      const result = await invoke<RecordingBatchResult>(
        "push_recordings_to_anki",
        { filePaths },
      );
      applyBootstrap(result.bootstrap);
      setRecordingActionMessage(result.message);
      if (
        result.status === "unavailable" ||
        result.message.toLowerCase().includes("anki is currently offline")
      ) {
        showWarning(result.message);
      }
    } catch (error) {
      const message =
        error instanceof Error
          ? error.message
          : "The recordings could not be pushed to Anki.";
      if (message.toLowerCase().includes("anki")) {
        showWarning(message);
      }
      setLoadError(
        message,
      );
    } finally {
      setBusyAction(null);
    }
  }

  async function transcribeRecordings(filePaths: string[]) {
    try {
      setBusyAction("transcribeRecording");
      await persistSettingsIfNeeded();
      const result = await invoke<RecordingBatchResult>("transcribe_recordings", {
        filePaths,
      });
      applyBootstrap(result.bootstrap);
      setRecordingActionMessage(result.message);
    } catch (error) {
      setLoadError(
        error instanceof Error
          ? error.message
          : "The recordings could not be transcribed.",
      );
    } finally {
      setBusyAction(null);
    }
  }

  async function translateRecordings(filePaths: string[]) {
    try {
      setBusyAction("translateRecording");
      const result = await invoke<RecordingBatchResult>("translate_recordings", {
        filePaths,
      });
      applyBootstrap(result.bootstrap);
      setRecordingActionMessage(result.message);
    } catch (error) {
      setLoadError(
        error instanceof Error
          ? error.message
          : "The translation request could not be completed.",
      );
    } finally {
      setBusyAction(null);
    }
  }

  function renderDownloadBlock(kind: "runtime" | "model" | "ffmpeg") {
    if (bootstrap.modelDownload.kind !== kind) {
      return null;
    }

    if (
      bootstrap.modelDownload.status === "idle" &&
      bootstrap.modelDownload.targetPath === null
    ) {
      return null;
    }

    return (
      <div className="download-card">
        <div className="progress-track" aria-hidden="true">
          <div
            className="progress-fill"
            style={{
              width: `${Math.max(
                0,
                Math.min(100, bootstrap.modelDownload.progressPercent ?? 0),
              )}%`,
            }}
          />
        </div>
        <p className="microcopy">
          {bootstrap.modelDownload.message}{" "}
          {formatProgressBytes(
            bootstrap.modelDownload.downloadedBytes,
            bootstrap.modelDownload.totalBytes,
          )}
          {bootstrap.modelDownload.progressPercent !== null
            ? ` (${bootstrap.modelDownload.progressPercent.toFixed(1)}%)`
            : ""}
        </p>
        {bootstrap.modelDownload.targetPath ? (
          <p className="path-copy" title={bootstrap.modelDownload.targetPath}>
            {fileNameFromPath(bootstrap.modelDownload.targetPath)}
          </p>
        ) : null}
        {downloadIsActive ? (
          <div className="action-row compact-actions">
            <button
              type="button"
              className="secondary"
              onClick={() => void toggleDownloadPause()}
              disabled={
                bootstrap.modelDownload.status === "starting" ||
                bootstrap.modelDownload.status === "cancelling"
              }
            >
              {bootstrap.modelDownload.status === "paused"
                ? "Resume Download"
                : "Pause Download"}
            </button>
            <button
              type="button"
              className="ghost"
              onClick={() => void cancelDownload()}
              disabled={bootstrap.modelDownload.status === "cancelling"}
            >
              Cancel Download
            </button>
          </div>
        ) : null}
      </div>
    );
  }

  function renderUpdateResult(result: WhisperAssetUpdateResult | null) {
    if (!result) {
      return null;
    }

    return (
      <div className={`update-card ${result.status}`}>
        <strong>{result.message}</strong>
        {result.currentVersion || result.latestVersion ? (
          <p className="microcopy">
            Current: {result.currentVersion ?? "Unknown"}{" "}
            {result.latestVersion ? `| Latest: ${result.latestVersion}` : ""}
          </p>
        ) : null}
      </div>
    );
  }

  function renderAnkiFieldSelect(field: keyof AnkiFieldMapping, label: string) {
    const currentValue = settingsDraft.anki.fields[field];
    const fieldOptions = ankiCatalog.fields;

    return (
      <label className="field">
        <span>{label}</span>
        <select
          value={currentValue}
          onChange={(event) =>
            updateSettings({
              anki: {
                fields: {
                  [field]: event.currentTarget.value,
                },
              },
            })
          }
        >
          <option value="">Not mapped</option>
          {currentValue && !fieldOptions.includes(currentValue) ? (
            <option value={currentValue}>{currentValue}</option>
          ) : null}
          {fieldOptions.map((fieldName) => (
            <option key={fieldName} value={fieldName}>
              {fieldName}
            </option>
          ))}
        </select>
      </label>
    );
  }

  return (
    <main className="app-shell">
      <nav className="tab-strip" aria-label="Sections">
        <button
          type="button"
          className={`tab-button ${activeTab === "recorder" ? "active" : ""}`}
          onClick={() => setActiveTab("recorder")}
          aria-pressed={activeTab === "recorder"}
        >
          Recorder
        </button>
        <button
          type="button"
          className={`tab-button ${activeTab === "settings" ? "active" : ""}`}
          onClick={() => setActiveTab("settings")}
          aria-pressed={activeTab === "settings"}
        >
          Settings
        </button>
      </nav>

      {loadError ? (
        <section className="banner banner-error">{loadError}</section>
      ) : null}

      {appWarning ? (
        <section className="warning-toast" role="alert">
          <div>
            <strong>Warning</strong>
            <p>{appWarning.message}</p>
          </div>
          <button
            type="button"
            className="ghost"
            onClick={() => setAppWarning(null)}
            aria-label="Dismiss warning"
          >
            Close
          </button>
        </section>
      ) : null}

      {showBusyOverlay ? (
        <section className="busy-panel">
          <div className="busy-spinner" aria-hidden="true" />
          <div>
            <p className="panel-kicker">Working</p>
            <strong>{busyOverlayLabel}</strong>
            <p className="microcopy">{bootstrap.shell.statusText}</p>
          </div>
        </section>
      ) : null}

      <section className="workspace">
        <section className="content-column">
          {activeTab === "recorder" ? (
            <div className="recorder-view">
              <article className="panel panel-primary">
                <header className="panel-header">
                  <div>
                    <p className="panel-kicker">Recorder</p>
                    <h2>System Audio</h2>
                  </div>
                  <div className="panel-actions">
                    <TooltipBadge
                      label="Shortcuts"
                      description={hotkeyTooltip}
                    />
                  </div>
                </header>

                <div className="recorder-topline">
                  <div className="timer-block">
                    <span className="hint-label">Elapsed</span>
                    <strong>{formatDuration(elapsedRecordingMs)}</strong>
                  </div>
                  <div className="status-stack" title={bootstrap.shell.statusText}>
                    <span className="hint-label">Status</span>
                    <strong>
                      {bootstrap.shell.phase === "idle"
                        ? "Ready"
                        : bootstrap.shell.phase === "recording"
                          ? "Recording"
                          : bootstrap.shell.phase === "saving"
                            ? "Saving"
                            : bootstrap.shell.phase === "transcribing"
                              ? "Transcribing"
                              : bootstrap.shell.statusText}
                    </strong>
                  </div>
                </div>

                <div className="action-row">
                  <button
                    type="button"
                    onClick={() => void startRecording()}
                    disabled={recorderBusy}
                  >
                    Start Recording
                  </button>
                  <button
                    type="button"
                    className="secondary"
                    onClick={() => void stopRecording()}
                    disabled={!isRecording || busyAction === "stop"}
                  >
                    Stop Recording
                  </button>
                  <button
                    type="button"
                    className="ghost"
                    onClick={() => void hideToTray()}
                    disabled={busyAction !== null}
                  >
                    Hide To Tray
                  </button>
                </div>
              </article>

              <article className="panel recent-panel">
                <header className="panel-header">
                  <div>
                    <p className="panel-kicker">Recent Output</p>
                    <h2>Saved Recordings</h2>
                  </div>
                </header>

                {recordingActionMessage ? (
                  <p className="microcopy">{recordingActionMessage}</p>
                ) : null}

                {bootstrap.recentRecordings.length > 0 ? (
                  <>
                    <div
                      className="recording-filter-tabs"
                      role="tablist"
                      aria-label="Saved recording filters"
                    >
                      {recordingFilterTabs.map((tab) => (
                        <button
                          key={tab.id}
                          type="button"
                          className={`recording-filter-tab ${
                            recordingFilter === tab.id ? "active" : ""
                          }`}
                          role="tab"
                          aria-selected={recordingFilter === tab.id}
                          onClick={() => setRecordingFilter(tab.id)}
                        >
                          <span>{tab.label}</span>
                          <strong>{tab.count}</strong>
                        </button>
                      ))}
                    </div>

                    <div className="recording-toolbar">
                      <span className="selection-summary">
                        {visibleSelectedPaths.length > 0
                          ? `${visibleSelectedPaths.length} selected`
                          : `${visibleRecordings.length} shown`}
                      </span>
                      <div className="recording-toolbar-actions">
                        {selectedUntranscribedRecordings.length > 0 ? (
                          <button
                            type="button"
                            className="secondary"
                            onClick={() =>
                              void transcribeRecordings(
                                selectedUntranscribedRecordings.map(
                                  (recording) => recording.filePath,
                                ),
                              )
                            }
                            disabled={busyAction === "transcribeRecording"}
                          >
                            Transcribe Selected
                          </button>
                        ) : null}
                        {recordingFilter === "needsTranscription" &&
                        untranscribedRecordings.length > 0 ? (
                          <button
                            type="button"
                            className="secondary"
                            onClick={() =>
                              void transcribeRecordings(
                                untranscribedRecordings.map(
                                  (recording) => recording.filePath,
                                ),
                              )
                            }
                            disabled={busyAction === "transcribeRecording"}
                          >
                            Transcribe All
                          </button>
                        ) : null}
                        {selectedPushableRecordings.length > 0 ? (
                          <button
                            type="button"
                            className="secondary"
                            onClick={() =>
                              void pushRecordingsToAnki(
                                selectedPushableRecordings.map(
                                  (recording) => recording.filePath,
                                ),
                              )
                            }
                            disabled={busyAction === "pushAnki"}
                          >
                            Push Selected
                          </button>
                        ) : null}
                        {recordingFilter === "needsAnki" &&
                        pushableRecordings.length > 0 ? (
                          <button
                            type="button"
                            className="secondary"
                            onClick={() =>
                              void pushRecordingsToAnki(
                                pushableRecordings.map(
                                  (recording) => recording.filePath,
                                ),
                              )
                            }
                            disabled={busyAction === "pushAnki"}
                          >
                            Push All
                          </button>
                        ) : null}
                        {selectedUntranslatedRecordings.length > 0 ? (
                          <button
                            type="button"
                            className="secondary"
                            onClick={() =>
                              void translateRecordings(
                                selectedUntranslatedRecordings.map(
                                  (recording) => recording.filePath,
                                ),
                              )
                            }
                            disabled={busyAction === "translateRecording"}
                          >
                            Translate Selected
                          </button>
                        ) : null}
                        {recordingFilter === "needsTranslation" &&
                        untranslatedRecordings.length > 0 ? (
                          <button
                            type="button"
                            className="secondary"
                            onClick={() =>
                              void translateRecordings(
                                untranslatedRecordings.map(
                                  (recording) => recording.filePath,
                                ),
                              )
                            }
                            disabled={busyAction === "translateRecording"}
                          >
                            Translate All
                          </button>
                        ) : null}
                        {visibleSelectedPaths.length > 0 ? (
                          <>
                            <button
                              type="button"
                              className="ghost danger-action"
                              onClick={() =>
                                void deleteRecordings(visibleSelectedPaths)
                              }
                              disabled={busyAction === "deleteRecording"}
                            >
                              Delete Selected
                            </button>
                            <button
                              type="button"
                              className="ghost"
                              onClick={clearRecordingSelection}
                            >
                              Clear
                            </button>
                          </>
                        ) : null}
                      </div>
                    </div>
                  </>
                ) : null}

                {bootstrap.recentRecordings.length === 0 ? (
                  <p className="empty-state">No recordings yet</p>
                ) : visibleRecordings.length === 0 ? (
                  <p className="empty-state">No recordings in this status</p>
                ) : (
                  <div className="recording-list">
                    {visibleRecordings.map((recording) => (
                      <article className="recording-item" key={recording.filePath}>
                        <div className="recording-head">
                          <label className="recording-select">
                            <input
                              type="checkbox"
                              checked={selectedRecordingSet.has(recording.filePath)}
                              onChange={() =>
                                toggleRecordingSelection(recording.filePath)
                              }
                              aria-label={`Select ${recording.fileName}`}
                            />
                            <strong>{recording.fileName}</strong>
                          </label>
                          <span>{formatDuration(recording.durationMs)}</span>
                        </div>
                        <div className="recording-meta">
                          <span>{formatBytes(recording.bytesWritten)}</span>
                          <span>{formatTimestamp(recording.createdAtMs)}</span>
                        </div>
                        <div
                          className="recording-state-row"
                          title={
                            recording.transcriptPath
                              ? `Audio: ${recording.filePath}\nTranscript: ${recording.transcriptPath}`
                              : `Audio: ${recording.filePath}`
                          }
                        >
                          <span className="recording-state">
                            {recording.transcriptPath
                              ? "Audio + transcript"
                              : "Audio only"}
                          </span>
                          {recording.ankiNoteId !== null ? (
                            <span
                              className="recording-state success-state"
                              title={
                                recording.ankiDeckName
                                  ? `Pushed to ${recording.ankiDeckName}${
                                      recording.ankiNoteType
                                        ? ` / ${recording.ankiNoteType}`
                                        : ""
                                    }`
                                  : "Pushed to Anki"
                              }
                            >
                              {recording.ankiDeckName
                                ? `Anki: ${recording.ankiDeckName}`
                                : "In Anki"}
                            </span>
                          ) : null}
                          {recording.translationPath !== null ? (
                            <span className="recording-state success-state">
                              Translated
                            </span>
                          ) : null}
                        </div>
                        <div className="recording-actions">
                          <button
                            type="button"
                            className="ghost"
                            onClick={() => void playRecording(recording.filePath)}
                            disabled={busyAction === "playRecording"}
                          >
                            Play
                          </button>
                          {!recording.transcriptPath ? (
                            <button
                              type="button"
                              className="secondary"
                              onClick={() =>
                                void transcribeRecordings([recording.filePath])
                              }
                              disabled={busyAction === "transcribeRecording"}
                            >
                              Transcribe
                            </button>
                          ) : null}
                          {recording.transcriptPath &&
                          !recordingPushedToCurrentAnkiTarget(recording) ? (
                            <button
                              type="button"
                              className="secondary"
                              onClick={() =>
                                void pushRecordingsToAnki([recording.filePath])
                              }
                              disabled={busyAction === "pushAnki"}
                            >
                              {recording.ankiNoteId !== null
                                ? "Push Again"
                                : "Push"}
                            </button>
                          ) : null}
                          {recording.transcriptPath &&
                          recording.translationPath === null ? (
                            <button
                              type="button"
                              className="secondary"
                              onClick={() =>
                                void translateRecordings([recording.filePath])
                              }
                              disabled={busyAction === "translateRecording"}
                            >
                              Translate
                            </button>
                          ) : null}
                          <button
                            type="button"
                            className="ghost danger-action"
                            onClick={() => void deleteRecording(recording.filePath)}
                            disabled={busyAction === "deleteRecording"}
                          >
                            Delete
                          </button>
                        </div>
                      </article>
                    ))}
                  </div>
                )}
              </article>
            </div>
          ) : null}

          {activeTab === "settings" ? (
            <div className="settings-scroll">
              <div className="settings-overview-grid">
                <article className="panel settings-card">
                <header className="panel-header">
                  <div>
                    <p className="panel-kicker">Settings</p>
                    <h2>App Preferences</h2>
                  </div>
                </header>

                {autosaveState === "error" ? (
                  <p className="autosave-error" role="alert">
                    {autosaveMessage}
                  </p>
                ) : null}

                <div className="settings-grid">
                  <label className="field">
                    <span>Appearance</span>
                    <select
                      value={settingsDraft.theme}
                      onChange={(event) =>
                        updateSettings({
                          theme: event.currentTarget.value as ThemePreference,
                        })
                      }
                    >
                      <option value="system">Use system setting</option>
                      <option value="light">Light</option>
                      <option value="dark">Dark</option>
                    </select>
                  </label>

                  <label className="field">
                    <span>Recording output folder</span>
                    <div className="input-with-action">
                      <input
                        type="text"
                        value={settingsDraft.outputDirectory}
                        onChange={(event) =>
                          updateSettings({
                            outputDirectory: event.currentTarget.value,
                          })
                        }
                        placeholder="Choose where WAV files are stored"
                      />
                      <button
                        type="button"
                        className="ghost"
                        onClick={() => void browseForDirectory("outputDirectory")}
                        disabled={busyAction === "browse"}
                      >
                        Browse
                      </button>
                    </div>
                  </label>

                  <label className="field">
                    <span>Model and asset folder</span>
                    <div className="input-with-action">
                      <input
                        type="text"
                        value={settingsDraft.assetDirectory}
                        onChange={(event) =>
                          updateSettings({
                            assetDirectory: event.currentTarget.value,
                          })
                        }
                        placeholder="Choose where Whisper runtime and model assets live"
                      />
                      <button
                        type="button"
                        className="ghost"
                        onClick={() => void browseForDirectory("assetDirectory")}
                        disabled={busyAction === "browse"}
                      >
                        Browse
                      </button>
                    </div>
                  </label>

                  <div className="toggle-grid">
                    <label className="toggle">
                      <input
                        type="checkbox"
                        checked={settingsDraft.features.transcription}
                        onChange={(event) =>
                          updateSettings({
                            features: {
                              transcription: event.currentTarget.checked,
                            },
                          })
                        }
                      />
                      <span>Enable transcription</span>
                    </label>

                    <label className="toggle">
                      <input
                        type="checkbox"
                        checked={settingsDraft.launchAtLogin}
                        onChange={(event) =>
                          updateSettings({
                            launchAtLogin: event.currentTarget.checked,
                          })
                        }
                      />
                      <span>Launch with Windows</span>
                    </label>

                    <label className="toggle">
                      <input
                        type="checkbox"
                        checked={settingsDraft.startMinimized}
                        onChange={(event) =>
                          updateSettings({
                            startMinimized: event.currentTarget.checked,
                          })
                        }
                      />
                      <span>Start minimized to tray</span>
                    </label>
                  </div>
                </div>
                </article>

                <article className="panel settings-card">
                <header className="panel-header">
                  <div>
                    <p className="panel-kicker">Whisper Setup</p>
                    <h2>Whisper</h2>
                  </div>
                  <TooltipBadge
                    label={whisperStatusLabel(bootstrap.whisperDetection.status)}
                    description={bootstrap.whisperDetection.message}
                  />
                </header>

                <div className="meta-list compact-meta-list">
                  <div
                    title={bootstrap.whisperDetection.executablePath || "Not installed"}
                  >
                    <span className="hint-label">Runtime</span>
                    <strong>
                      {bootstrap.whisperDetection.cliReady
                        ? `Ready (${activeRuntimeVersion})`
                        : "Missing"}
                    </strong>
                  </div>
                  <div
                    title={bootstrap.whisperDetection.modelPath || "Not installed"}
                  >
                    <span className="hint-label">Model</span>
                    <strong>
                      {bootstrap.whisperDetection.modelReady ? "Ready" : "Missing"}
                    </strong>
                  </div>
                  <div>
                    <span className="hint-label">Language</span>
                    <strong>{settingsDraft.whisper.language}</strong>
                  </div>
                </div>
                </article>
              </div>

              <div className="whisper-config-grid">
                <article className="panel settings-card">
                  <header className="panel-header">
                    <div>
                      <p className="panel-kicker">Runtime</p>
                      <h2>Whisper CLI</h2>
                    </div>
                    <TooltipBadge
                      label="?"
                      description="Paste a path if whisper-cli is already installed somewhere else, or let the app download and manage the recommended Windows runtime."
                    />
                  </header>

                  {installedRuntimeVersions.length > 0 ? (
                    <label className="field runtime-version-field">
                      <span>Active runtime</span>
                      <select
                        value={activeRuntimeVersion}
                        onChange={(event) =>
                          updateSettings({
                            whisper: {
                              runtimeVersion: event.currentTarget.value,
                              cliPath: "",
                            },
                          })
                        }
                        disabled={manualRuntimeOverride}
                        title={
                          manualRuntimeOverride
                            ? "Clear the manual runtime override to use app-managed versions."
                            : "Choose any installed app-managed Whisper runtime."
                        }
                      >
                        {installedRuntimeVersions.map((version) => (
                          <option key={version} value={version}>
                            {version}
                          </option>
                        ))}
                      </select>
                    </label>
                  ) : null}

                  <details className="disclosure">
                    <summary>Manual runtime override</summary>
                    <label className="field">
                      <span>whisper-cli path</span>
                      <div className="input-with-action">
                        <input
                          type="text"
                          value={resolvedCliPath}
                          onChange={(event) =>
                            updateSettings({
                              whisper: {
                                cliPath: event.currentTarget.value,
                              },
                            })
                          }
                          placeholder="whisper-cli path"
                        />
                        <button
                          type="button"
                          className="ghost"
                          onClick={() => void browseForFile("cliPath")}
                          disabled={busyAction === "browse"}
                        >
                          Browse
                        </button>
                      </div>
                    </label>
                  </details>

                  <div className="download-section">
                    {runtimeInstalled ? (
                      <div className="installed-card">
                        <div className="installed-row">
                          <strong>Runtime ready</strong>
                          {bootstrap.whisperDetection.cliManaged ? (
                            <div className="action-row inline-actions">
                              <button
                                type="button"
                                className="secondary"
                                onClick={() => void checkRuntimeUpdate()}
                                disabled={busyAction === "checkRuntimeUpdate"}
                              >
                                Check for Updates
                              </button>
                            </div>
                          ) : null}
                        </div>
                        {renderUpdateResult(runtimeUpdateResult)}
                        {runtimeUpdateVersion ? (
                          <div className="action-row compact-actions">
                            <button
                              type="button"
                              onClick={() =>
                                void downloadRuntimeVersion(runtimeUpdateVersion)
                              }
                              disabled={
                                downloadIsActive ||
                                busyAction === "downloadRuntime"
                              }
                            >
                              Download {runtimeUpdateVersion}
                            </button>
                          </div>
                        ) : null}
                      </div>
                    ) : (
                      <div className="action-row inline-actions">
                        <button
                          type="button"
                          onClick={() => void downloadRecommendedRuntime()}
                          disabled={
                            downloadIsActive || busyAction === "downloadRuntime"
                          }
                        >
                          Download Recommended Runtime
                        </button>
                      </div>
                    )}
                    {renderDownloadBlock("runtime")}
                  </div>
                </article>

                <article className="panel settings-card">
                  <header className="panel-header">
                    <div>
                      <p className="panel-kicker">Model</p>
                      <h2>Whisper Model</h2>
                    </div>
                    <TooltipBadge
                      label="?"
                      description="Choose a model file manually, or let the app download the recommended multilingual model into your selected asset folder."
                    />
                  </header>

                <div className="settings-grid">
                  <label className="field">
                    <span>Managed model</span>
                    <select
                      value={settingsDraft.whisper.modelChoice}
                      onChange={(event) =>
                        updateSettings({
                          whisper: {
                            modelChoice: event.currentTarget.value,
                          },
                        })
                      }
                    >
                      {MODEL_OPTIONS.map((option) => (
                        <option key={option.id} value={option.id}>
                          {option.label}
                        </option>
                      ))}
                    </select>
                  </label>

                  <label className="field">
                    <span>Language</span>
                    <select
                      value={selectedLanguageCode}
                      onChange={(event) =>
                        updateSettings({
                          whisper: {
                            language: event.currentTarget.value,
                          },
                        })
                      }
                    >
                      {!selectedLanguageKnown ? (
                        <option value={selectedLanguageCode}>
                          Custom ({selectedLanguageCode})
                        </option>
                      ) : null}
                      {LANGUAGE_OPTIONS.map((option) => (
                        <option key={option.code} value={option.code}>
                          {option.label} ({option.code})
                        </option>
                      ))}
                    </select>
                  </label>
                </div>

                <div className="model-summary" title={selectedModel.description}>
                  <strong>{selectedModel.label}</strong>
                  <span>
                    {selectedModel.diskSize} · {selectedModel.memoryUsage} RAM
                  </span>
                </div>

                <details className="disclosure">
                  <summary>Manual model override</summary>
                  <label className="field">
                    <span>GGML model path</span>
                    <div className="input-with-action">
                      <input
                        type="text"
                        value={resolvedModelPath}
                        onChange={(event) =>
                          updateSettings({
                            whisper: {
                              modelPath: event.currentTarget.value,
                            },
                          })
                        }
                        placeholder="GGML model path"
                      />
                      <button
                        type="button"
                        className="ghost"
                        onClick={() => void browseForFile("modelPath")}
                        disabled={busyAction === "browse"}
                      >
                        Browse
                      </button>
                    </div>
                  </label>
                </details>

                <div className="download-section">
                  {modelInstalled ? (
                    <div className="installed-card">
                      <div className="installed-row">
                        <strong>Model ready</strong>
                        {bootstrap.whisperDetection.modelManaged ? (
                          <div className="action-row inline-actions">
                            <button
                              type="button"
                              className="secondary"
                              onClick={() => void checkModelUpdate()}
                              disabled={busyAction === "checkModelUpdate"}
                            >
                              Check for Updates
                            </button>
                          </div>
                        ) : null}
                      </div>
                      {renderUpdateResult(modelUpdateResult)}
                    </div>
                  ) : (
                    <div className="action-row inline-actions">
                      <button
                        type="button"
                        className="secondary"
                        onClick={() => void downloadRecommendedModel()}
                        disabled={downloadIsActive || busyAction === "downloadModel"}
                      >
                        Download {selectedModel.label} Model
                      </button>
                    </div>
                  )}
                  {renderDownloadBlock("model")}
                </div>
                </article>

                <article className="panel settings-card settings-card-wide">
                  <header className="panel-header">
                    <div>
                      <p className="panel-kicker">Storage</p>
                      <h2>MP3 Compression</h2>
                    </div>
                    <TooltipBadge
                      label={
                        bootstrap.ffmpegDetection.status === "ready"
                          ? "Ready"
                          : "Missing"
                      }
                      description={bootstrap.ffmpegDetection.message}
                    />
                  </header>

                  <div
                    className={`update-card ${
                      bootstrap.ffmpegDetection.status === "ready"
                        ? "current"
                        : "available"
                    }`}
                  >
                    <strong>{bootstrap.ffmpegDetection.message}</strong>
                    <p className="microcopy">
                      Wonder of U records WAV for reliable Whisper transcription,
                      then compresses the saved audio to MP3 after the transcript is
                      created. If FFmpeg is missing or conversion fails, the WAV is
                      kept.
                    </p>
                    {bootstrap.ffmpegDetection.executablePath ? (
                      <p
                        className="path-copy"
                        title={bootstrap.ffmpegDetection.executablePath}
                      >
                        {fileNameFromPath(bootstrap.ffmpegDetection.executablePath)}
                      </p>
                    ) : null}
                  </div>

                  {bootstrap.ffmpegDetection.status !== "ready" ? (
                    <div className="action-row inline-actions">
                      <button
                        type="button"
                        className="secondary"
                        onClick={() => void downloadRecommendedFfmpeg()}
                        disabled={downloadIsActive || busyAction === "downloadFfmpeg"}
                      >
                        Download FFmpeg
                      </button>
                    </div>
                  ) : null}
                  {renderDownloadBlock("ffmpeg")}
                </article>
              </div>

              <article className="panel anki-panel settings-card settings-card-wide">
                <header className="panel-header">
                  <div>
                    <p className="panel-kicker">Anki</p>
                    <h2>Card Mapping</h2>
                  </div>
                  <div className="panel-actions">
                    <TooltipBadge
                      label={ankiCatalog.status === "ready" ? "Ready" : "Setup"}
                      description={ankiCatalog.message}
                    />
                    <button
                      type="button"
                      className="secondary"
                      onClick={() => void refreshAnkiCatalog()}
                      disabled={busyAction === "loadAnki"}
                    >
                      Refresh Anki
                    </button>
                  </div>
                </header>

                <div
                  className={`update-card ${
                    ankiCatalog.status === "ready"
                      ? "current"
                      : ankiCatalog.status === "offline"
                        ? "error"
                        : ""
                  }`}
                >
                  <strong>{ankiCatalog.message}</strong>
                  {ankiCatalog.version !== null ? (
                    <p className="microcopy">
                      AnkiConnect version {ankiCatalog.version}
                    </p>
                  ) : null}
                </div>

                <div className="settings-grid anki-grid">
                  <label className="field">
                    <span>Deck</span>
                    <select
                      value={settingsDraft.anki.deckName}
                      onChange={(event) =>
                        updateSettings({
                          anki: {
                            deckName: event.currentTarget.value,
                          },
                        })
                      }
                    >
                      <option value="">Choose deck</option>
                      {settingsDraft.anki.deckName &&
                      !ankiCatalog.decks.includes(settingsDraft.anki.deckName) ? (
                        <option value={settingsDraft.anki.deckName}>
                          {settingsDraft.anki.deckName}
                        </option>
                      ) : null}
                      {ankiCatalog.decks.map((deck) => (
                        <option key={deck} value={deck}>
                          {deck}
                        </option>
                      ))}
                    </select>
                  </label>

                  <label className="field">
                    <span>Note type</span>
                    <select
                      value={settingsDraft.anki.noteType}
                      onChange={(event) => {
                        const noteType = event.currentTarget.value;
                        updateSettings({
                          anki: {
                            noteType,
                            fields: {
                              transcription: "",
                              audio: "",
                              translation: "",
                              sourcePath: "",
                              createdAt: "",
                            },
                          },
                        });
                        if (noteType) {
                          void refreshAnkiCatalog(noteType);
                        }
                      }}
                    >
                      <option value="">Choose note type</option>
                      {settingsDraft.anki.noteType &&
                      !ankiCatalog.noteTypes.includes(settingsDraft.anki.noteType) ? (
                        <option value={settingsDraft.anki.noteType}>
                          {settingsDraft.anki.noteType}
                        </option>
                      ) : null}
                      {ankiCatalog.noteTypes.map((noteType) => (
                        <option key={noteType} value={noteType}>
                          {noteType}
                        </option>
                      ))}
                    </select>
                  </label>

                  {renderAnkiFieldSelect("transcription", "Transcript field")}
                  {renderAnkiFieldSelect("audio", "Audio field")}
                  {renderAnkiFieldSelect("translation", "Translation field")}
                  {renderAnkiFieldSelect("sourcePath", "Source path field")}
                  {renderAnkiFieldSelect("createdAt", "Created-at field")}
                </div>
              </article>
            </div>
          ) : null}
        </section>
      </section>
    </main>
  );
}

export default App;
