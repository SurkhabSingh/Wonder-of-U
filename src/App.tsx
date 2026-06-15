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

type WhisperSettings = {
  cliPath: string;
  modelPath: string;
  modelChoice: string;
  language: string;
};

type ThemePreference = "system" | "light" | "dark";

type AppSettings = {
  outputDirectory: string;
  assetDirectory: string;
  whisper: WhisperSettings;
  features: FeatureSettings;
  theme: ThemePreference;
  launchAtLogin: boolean;
  startMinimized: boolean;
};

type RecentRecording = {
  fileName: string;
  filePath: string;
  transcriptPath: string | null;
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
  cliReady: boolean;
  modelReady: boolean;
  cliManaged: boolean;
  modelManaged: boolean;
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
  modelDownload: ModelDownloadSnapshot;
  logPath: string;
};

type BusyAction =
  | "start"
  | "stop"
  | "hide"
  | "browse"
  | "downloadModel"
  | "downloadRuntime"
  | "checkRuntimeUpdate"
  | "checkModelUpdate"
  | null;

type AutosaveState = "idle" | "saving" | "error";
type AppTab = "recorder" | "settings" | "whisper";

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
      modelChoice: "small",
      language: "auto",
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
    cliReady: false,
    modelReady: false,
    cliManaged: false,
    modelManaged: false,
    message:
      "Add or download whisper-cli and a Whisper model to enable offline transcription.",
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
  const [clockMs, setClockMs] = useState(() => Date.now());
  const [activeTab, setActiveTab] = useState<AppTab>("recorder");
  const [runtimeUpdateResult, setRuntimeUpdateResult] =
    useState<WhisperAssetUpdateResult | null>(null);
  const [modelUpdateResult, setModelUpdateResult] =
    useState<WhisperAssetUpdateResult | null>(null);
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
  }, [settingsDraft.assetDirectory, settingsDraft.whisper.cliPath]);

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

  const phaseTone = useMemo(() => {
    switch (bootstrap.shell.phase) {
      case "recording":
        return "recording";
      case "saving":
      case "transcribing":
      case "downloading-model":
        return "saving";
      case "error":
        return "error";
      default:
        return "idle";
    }
  }, [bootstrap.shell.phase]);

  const isRecording = bootstrap.shell.phase === "recording";
  const isSaving = bootstrap.shell.phase === "saving";
  const isTranscribing = bootstrap.shell.phase === "transcribing";
  const recorderBusy =
    isRecording ||
    isSaving ||
    isTranscribing ||
    busyAction === "start" ||
    busyAction === "stop";
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

  function updateSettings(
    update: Partial<Omit<AppSettings, "features" | "whisper">> & {
      features?: Partial<FeatureSettings>;
      whisper?: Partial<WhisperSettings>;
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

      return {
        ...current,
        ...update,
        whisper: nextWhisper,
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

  async function downloadRecommendedRuntime() {
    try {
      setBusyAction("downloadRuntime");
      setRuntimeUpdateResult(null);
      await persistSettingsIfNeeded();
      const nextBootstrap = await invoke<AppBootstrap>(
        "download_recommended_whisper_runtime",
      );
      applyBootstrap(nextBootstrap);
      setActiveTab("whisper");
    } catch (error) {
      setLoadError(
        error instanceof Error
          ? error.message
          : "The recommended Whisper runtime could not be prepared.",
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
      setActiveTab("whisper");
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

  function renderDownloadBlock(kind: "runtime" | "model", label: string) {
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
          <p className="path-copy">{bootstrap.modelDownload.targetPath}</p>
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
        ) : (
          <p className="download-caption">{label} download status is shown here.</p>
        )}
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

  return (
    <main className="app-shell">
      <section className="hero">
        <div>
          <p className="eyebrow">Local Capture And Transcription</p>
          <h1>Wonder of U Desktop</h1>
          <p className="lede">
            The recorder now handles real system-audio capture plus offline
            Whisper transcription, while the setup flow stays tighter: use a
            manual override when you already have `whisper-cli`, or let the app
            manage the runtime and model for you.
          </p>
        </div>

        <div
          className={`state-chip ${phaseTone}`}
          title={bootstrap.shell.statusText}
        >
          <span className="state-chip-label">Recorder State</span>
          <strong>{bootstrap.shell.phase}</strong>
          <span className="state-chip-meta">
            {bootstrap.shell.currentRecordingName || "Ready for the next capture"}
          </span>
        </div>
      </section>

      {loadError ? (
        <section className="banner banner-error">{loadError}</section>
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
        <aside className="sidebar">
          <button
            type="button"
            className={`tab-button ${activeTab === "recorder" ? "active" : ""}`}
            onClick={() => setActiveTab("recorder")}
          >
            Main Recorder
          </button>
          <button
            type="button"
            className={`tab-button ${activeTab === "settings" ? "active" : ""}`}
            onClick={() => setActiveTab("settings")}
          >
            Settings
          </button>
          <button
            type="button"
            className={`tab-button ${activeTab === "whisper" ? "active" : ""}`}
            onClick={() => setActiveTab("whisper")}
          >
            Whisper Setup
          </button>

          <div className="sidebar-note">
            <span className="hint-label">Auto Save</span>
            <strong
              className={
                autosaveState === "error"
                  ? "status-error"
                  : autosaveState === "saving"
                    ? "status-pending"
                    : "status-ok"
              }
            >
              {autosaveState === "saving"
                ? "Saving..."
                : autosaveState === "error"
                  ? "Needs attention"
                  : "On"}
            </strong>
            <p className="microcopy">{autosaveMessage}</p>
          </div>
        </aside>

        <section className="content-column">
          {activeTab === "recorder" ? (
            <>
              <article className="panel panel-primary">
                <header className="panel-header">
                  <div>
                    <p className="panel-kicker">Recorder</p>
                    <h2>System Audio Capture</h2>
                  </div>
                  <div className="panel-actions">
                    <TooltipBadge
                      label="Shortcuts"
                      description={hotkeyTooltip}
                    />
                    <div className="metric">
                      <span>Transitions</span>
                      <strong>{bootstrap.shell.transitionCount}</strong>
                    </div>
                  </div>
                </header>

                <div className="recorder-topline">
                  <div className="timer-block">
                    <span className="hint-label">Elapsed</span>
                    <strong>{formatDuration(elapsedRecordingMs)}</strong>
                  </div>
                  <div className="status-stack">
                    <span className="hint-label">Status</span>
                    <strong>{bootstrap.shell.statusText}</strong>
                  </div>
                </div>

                <p className="microcopy">
                  Recordings always save to your chosen output folder first. If
                  transcript-based naming is enabled, the file names are updated
                  after Whisper finishes.
                </p>

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

                <div className="hint-row">
                  <div>
                    <span className="hint-label">Last Shortcut</span>
                    <strong>
                      {bootstrap.shell.lastShortcut || "No hotkey has fired yet"}
                    </strong>
                  </div>
                  <div>
                    <span className="hint-label">Last Saved File</span>
                    <strong>
                      {bootstrap.shell.lastOutputPath || "No recordings saved yet"}
                    </strong>
                  </div>
                  <div>
                    <span className="hint-label">Last Transcript</span>
                    <strong>
                      {bootstrap.shell.lastTranscriptPath ||
                        "No transcript saved yet"}
                    </strong>
                  </div>
                </div>
              </article>

              <article className="panel">
                <header className="panel-header">
                  <div>
                    <p className="panel-kicker">Recent Output</p>
                    <h2>Saved Recordings</h2>
                  </div>
                </header>

                {bootstrap.recentRecordings.length === 0 ? (
                  <p className="microcopy">
                    Your saved recordings will appear here after the first
                    successful system-audio capture.
                  </p>
                ) : (
                  <div className="recording-list">
                    {bootstrap.recentRecordings.map((recording) => (
                      <article className="recording-item" key={recording.filePath}>
                        <div className="recording-head">
                          <strong>{recording.fileName}</strong>
                          <span>{formatDuration(recording.durationMs)}</span>
                        </div>
                        <div className="recording-meta">
                          <span>{formatBytes(recording.bytesWritten)}</span>
                          <span>{formatTimestamp(recording.createdAtMs)}</span>
                        </div>
                        <p className="path-copy">{recording.filePath}</p>
                        {recording.transcriptPath ? (
                          <p className="path-copy">
                            Transcript: {recording.transcriptPath}
                          </p>
                        ) : null}
                      </article>
                    ))}
                  </div>
                )}
              </article>
            </>
          ) : null}

          {activeTab === "settings" ? (
            <article className="panel">
              <header className="panel-header">
                <div>
                  <p className="panel-kicker">Settings</p>
                  <h2>App Preferences</h2>
                </div>
                <TooltipBadge
                  label="Auto-saved"
                  description="Every change is saved automatically after a short delay. Recording actions also commit your latest settings before they run."
                />
              </header>

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
                  <small className="field-help">
                    System follows your operating system. Light and dark choices
                    override it on this device.
                  </small>
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

                <div className="pill-row">
                  <span className="pill">
                    Output: {settingsDraft.outputDirectory || "Not set yet"}
                  </span>
                  <span className="pill">
                    Assets: {settingsDraft.assetDirectory || "Not set yet"}
                  </span>
                  <span className="pill">
                    Log file: {bootstrap.logPath || "Will appear after startup"}
                  </span>
                </div>
              </div>
            </article>
          ) : null}

          {activeTab === "whisper" ? (
            <>
              <article className="panel">
                <header className="panel-header">
                  <div>
                    <p className="panel-kicker">Whisper Setup</p>
                    <h2>Offline Transcription</h2>
                  </div>
                  <TooltipBadge
                    label={whisperStatusLabel(bootstrap.whisperDetection.status)}
                    description={bootstrap.whisperDetection.message}
                  />
                </header>

                <div className="meta-list compact-meta-list">
                  <div
                    title={`CLI source: ${bootstrap.whisperDetection.source || "none"}`}
                  >
                    <span className="hint-label">Active whisper-cli</span>
                    <strong>
                      {bootstrap.whisperDetection.executablePath ||
                        "No active whisper-cli path yet"}
                    </strong>
                  </div>
                  <div
                    title={`Model source: ${
                      bootstrap.whisperDetection.modelSource || "none"
                    }`}
                  >
                    <span className="hint-label">Active model</span>
                    <strong>
                      {bootstrap.whisperDetection.modelPath ||
                        "No active ggml model path yet"}
                    </strong>
                  </div>
                  <div title={bootstrap.whisperDetection.message}>
                    <span className="hint-label">Readiness</span>
                    <strong>{bootstrap.whisperDetection.message}</strong>
                  </div>
                </div>
              </article>

              <article className="panel">
                <header className="panel-header">
                  <div>
                    <p className="panel-kicker">Runtime</p>
                    <h2>Whisper CLI</h2>
                  </div>
                  <TooltipBadge
                    label="Help"
                    description="Paste a path if whisper-cli is already installed somewhere else, or let the app download and manage the recommended Windows runtime."
                  />
                </header>

                <div className="settings-grid">
                  <label className="field">
                    <span>Manual whisper-cli path</span>
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
                        placeholder="Optional override if whisper-cli is already installed elsewhere"
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
                </div>

                <div className="download-section">
                  {runtimeInstalled ? (
                    <div className="installed-card">
                      <strong>Whisper CLI is already available.</strong>
                      <p className="microcopy">
                        {bootstrap.whisperDetection.cliManaged
                          ? "The app-managed runtime is installed and ready."
                          : "A manual whisper-cli path is active."}
                      </p>
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
                      {renderUpdateResult(runtimeUpdateResult)}
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
                  {renderDownloadBlock("runtime", "Runtime")}
                </div>
              </article>

              <article className="panel">
                <header className="panel-header">
                  <div>
                    <p className="panel-kicker">Model</p>
                    <h2>Whisper Model</h2>
                  </div>
                  <TooltipBadge
                    label="Help"
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
                    <span>Whisper model file</span>
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
                        placeholder="Optional override for a specific ggml model file"
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

                  <label className="field">
                    <span>Whisper language</span>
                    <input
                      type="text"
                      value={settingsDraft.whisper.language}
                      onChange={(event) =>
                        updateSettings({
                          whisper: {
                            language: event.currentTarget.value,
                          },
                        })
                      }
                      placeholder="Use auto for auto-detect or enter a language code like ja"
                    />
                  </label>
                </div>

                <div className="installed-card">
                  <strong>
                    {selectedModel.label}: {selectedModel.diskSize} disk,{" "}
                    {selectedModel.memoryUsage} RAM
                  </strong>
                  <p className="microcopy">{selectedModel.description}</p>
                </div>

                <div className="download-section">
                  {modelInstalled ? (
                    <div className="installed-card">
                      <strong>Selected Whisper model is already available.</strong>
                      <p className="microcopy">
                        {bootstrap.whisperDetection.modelManaged
                          ? "The app-managed model for your current selection is installed."
                          : "A manual Whisper model path is active."}
                      </p>
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
                  {renderDownloadBlock("model", "Model")}
                </div>
              </article>
            </>
          ) : null}
        </section>
      </section>
    </main>
  );
}

export default App;
