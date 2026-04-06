import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

type RecorderPhase = "idle" | "recording" | "saving" | "error" | string;

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
};

type FeatureSettings = {
  transcription: boolean;
  translation: boolean;
  anki: boolean;
};

type AppSettings = {
  outputDirectory: string;
  assetDirectory: string;
  features: FeatureSettings;
  launchAtLogin: boolean;
  startMinimized: boolean;
};

type RecentRecording = {
  fileName: string;
  filePath: string;
  durationMs: number;
  bytesWritten: number;
  createdAtMs: number;
};

type AppBootstrap = {
  shell: ShellSnapshot;
  settings: AppSettings;
  recentRecordings: RecentRecording[];
  logPath: string;
};

type BusyAction = "save" | "start" | "stop" | "hide" | "browse" | null;

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
  },
  settings: {
    outputDirectory: "",
    assetDirectory: "",
    features: {
      transcription: true,
      translation: false,
      anki: false,
    },
    launchAtLogin: false,
    startMinimized: false,
  },
  recentRecordings: [],
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

function App() {
  const [bootstrap, setBootstrap] = useState<AppBootstrap>(DEFAULT_BOOTSTRAP);
  const [settingsDraft, setSettingsDraft] = useState<AppSettings>(
    DEFAULT_BOOTSTRAP.settings,
  );
  const [settingsDirty, setSettingsDirty] = useState(false);
  const [busyAction, setBusyAction] = useState<BusyAction>(null);
  const [recordingName, setRecordingName] = useState("");
  const [loadError, setLoadError] = useState("");
  const [clockMs, setClockMs] = useState(() => Date.now());
  const settingsDirtyRef = useRef(false);

  useEffect(() => {
    settingsDirtyRef.current = settingsDirty;
  }, [settingsDirty]);

  useEffect(() => {
    let mounted = true;

    async function loadBootstrap() {
      try {
        const nextBootstrap = await invoke<AppBootstrap>("get_app_bootstrap");
        if (!mounted) {
          return;
        }

        setBootstrap(nextBootstrap);
        setSettingsDraft(nextBootstrap.settings);
        setSettingsDirty(false);
        setLoadError("");
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
      (bootstrap.shell.phase !== "recording" && bootstrap.shell.phase !== "saving")
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
        return "saving";
      case "error":
        return "error";
      default:
        return "idle";
    }
  }, [bootstrap.shell.phase]);

  const isRecording = bootstrap.shell.phase === "recording";
  const isSaving = bootstrap.shell.phase === "saving";
  const recorderBusy = isRecording || isSaving || busyAction === "start" || busyAction === "stop";

  function applyBootstrap(nextBootstrap: AppBootstrap) {
    setBootstrap(nextBootstrap);
    setSettingsDraft(nextBootstrap.settings);
    setSettingsDirty(false);
    setLoadError("");
  }

  function updateSettings(
    update: Partial<Omit<AppSettings, "features">> & {
      features?: Partial<FeatureSettings>;
    },
  ) {
    setSettingsDraft((current) => {
      const nextFeatures: FeatureSettings = {
        ...current.features,
        ...(update.features ?? {}),
      };

      if (!nextFeatures.transcription) {
        nextFeatures.translation = false;
        nextFeatures.anki = false;
      }
      if (nextFeatures.translation || nextFeatures.anki) {
        nextFeatures.transcription = true;
      }

      return {
        ...current,
        ...update,
        features: nextFeatures,
      };
    });
    setSettingsDirty(true);
  }

  async function saveSettings() {
    try {
      setBusyAction("save");
      const nextBootstrap = await invoke<AppBootstrap>("save_settings", {
        settings: settingsDraft,
      });
      applyBootstrap(nextBootstrap);
    } catch (error) {
      setLoadError(
        error instanceof Error
          ? error.message
          : "The settings could not be saved.",
      );
    } finally {
      setBusyAction(null);
    }
  }

  async function startRecording() {
    try {
      setBusyAction("start");
      const nextBootstrap = await invoke<AppBootstrap>("start_recording", {
        requestedName: recordingName.trim() || null,
      });
      applyBootstrap(nextBootstrap);
      setRecordingName("");
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

  return (
    <main className="app-shell">
      <section className="hero">
        <div>
          <p className="eyebrow">Phase 1 + Phase 2</p>
          <h1>Wonder of U Desktop</h1>
          <p className="lede">
            The tray shell is now backed by real Windows system-audio capture,
            persisted app settings, structured logs, and a recent-recordings
            history. This is the production-minded base we will build Whisper,
            CTranslate2, and Anki on top of.
          </p>
        </div>

        <div className={`state-chip ${phaseTone}`}>
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

      <section className="grid">
        <article className="panel panel-primary">
          <header className="panel-header">
            <div>
              <p className="panel-kicker">Recorder</p>
              <h2>System Audio Capture</h2>
            </div>
            <div className="metric">
              <span>Transitions</span>
              <strong>{bootstrap.shell.transitionCount}</strong>
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

          <label className="field">
            <span>Optional recording name</span>
            <input
              type="text"
              placeholder="Leave blank to use recording_1, recording_2, ..."
              value={recordingName}
              onChange={(event) => setRecordingName(event.currentTarget.value)}
              disabled={recorderBusy}
            />
          </label>

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
          </div>
        </article>

        <article className="panel">
          <header className="panel-header">
            <div>
              <p className="panel-kicker">Background Controls</p>
              <h2>Global Hotkeys</h2>
            </div>
          </header>

          <dl className="shortcut-list">
            <div>
              <dt>Start recording</dt>
              <dd>{bootstrap.shell.hotkeys.start}</dd>
            </div>
            <div>
              <dt>Stop recording</dt>
              <dd>{bootstrap.shell.hotkeys.stop}</dd>
            </div>
            <div>
              <dt>Show window</dt>
              <dd>{bootstrap.shell.hotkeys.showWindow}</dd>
            </div>
          </dl>

          <div className="meta-list">
            <div>
              <span className="hint-label">Launch at login</span>
              <strong>{settingsDraft.launchAtLogin ? "Enabled" : "Disabled"}</strong>
            </div>
            <div>
              <span className="hint-label">Start minimized</span>
              <strong>{settingsDraft.startMinimized ? "Enabled" : "Disabled"}</strong>
            </div>
            <div>
              <span className="hint-label">Structured log</span>
              <strong>{bootstrap.logPath || "Will be created after startup"}</strong>
            </div>
          </div>
        </article>
      </section>

      <section className="grid lower-grid">
        <article className="panel">
          <header className="panel-header">
            <div>
              <p className="panel-kicker">Settings</p>
              <h2>App Preferences</h2>
            </div>
            <div className={`save-badge ${settingsDirty ? "dirty" : "clean"}`}>
              {settingsDirty ? "Unsaved" : "Saved"}
            </div>
          </header>

          <div className="settings-grid">
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
                  placeholder="Choose where Whisper and translation assets live"
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
                  checked={settingsDraft.features.translation}
                  onChange={(event) =>
                    updateSettings({
                      features: {
                        translation: event.currentTarget.checked,
                      },
                    })
                  }
                />
                <span>Enable translation after transcription</span>
              </label>

              <label className="toggle">
                <input
                  type="checkbox"
                  checked={settingsDraft.features.anki}
                  onChange={(event) =>
                    updateSettings({
                      features: {
                        anki: event.currentTarget.checked,
                      },
                    })
                  }
                />
                <span>Offer Anki card creation</span>
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
            </div>

            <div className="action-row">
              <button
                type="button"
                onClick={() => void saveSettings()}
                disabled={!settingsDirty || busyAction === "save"}
              >
                Save Settings
              </button>
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
              Your saved recordings will appear here after the first successful
              system-audio capture.
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
                </article>
              ))}
            </div>
          )}
        </article>
      </section>
    </main>
  );
}

export default App;
