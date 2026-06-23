import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import * as TooltipPrimitive from "@radix-ui/react-tooltip";
import { Toaster, toast } from "sonner";
import { PageSidebar } from "./components/layout/PageSidebar";
import { RecorderPage } from "./components/recorder/RecorderPage";
import { SavedRecordingsPage } from "./components/recordings/SavedRecordingsPage";
import { SettingsPages } from "./components/settings/SettingsPages";
import { BusyOverlay } from "./components/ui/BusyOverlay";
import {
  APP_SNAPSHOT_EVENT,
  DEFAULT_ANKI_CATALOG,
  DEFAULT_BOOTSTRAP,
  MODEL_OPTIONS,
  RECOMMENDED_RUNTIME_VERSION,
} from "./constants";
import { useRecordingActions } from "./hooks/useRecordingActions";
import { useRecordingLibrary } from "./hooks/useRecordingLibrary";
import { normalizeSelection, whisperStatusLabel } from "./lib/helpers";
import {
  activePageLabel,
  createSetupPages,
  createWorkflowPages,
} from "./lib/navigation";
import type {
  AnkiCatalog,
  AnkiFieldMapping,
  AppBootstrap,
  AppPage,
  AppSettings,
  AutosaveState,
  BusyAction,
  AnkiSettings,
  FeatureSettings,
  WhisperAssetUpdateResult,
  WhisperSettings,
} from "./types";

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
  const [activePage, setActivePage] = useState<AppPage>("recorder");
  const [runtimeUpdateResult, setRuntimeUpdateResult] =
    useState<WhisperAssetUpdateResult | null>(null);
  const [modelUpdateResult, setModelUpdateResult] =
    useState<WhisperAssetUpdateResult | null>(null);
  const [ankiCatalog, setAnkiCatalog] =
    useState<AnkiCatalog>(DEFAULT_ANKI_CATALOG);
  const [recordingActionMessage, setRecordingActionMessage] = useState("");
  const settingsDirtyRef = useRef(false);
  const currentDraftKeyRef = useRef("");
  const ankiAutoRefreshInFlightRef = useRef(false);
  const recordingToastStateRef = useRef({
    phase: DEFAULT_BOOTSTRAP.shell.phase,
    transitionCount: DEFAULT_BOOTSTRAP.shell.transitionCount,
  });

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

  function showWarning(message: string) {
    toast.warning(message, { duration: 5000 });
  }

  function showSuccess(message: string) {
    toast.success(message, { duration: 3500 });
  }

  function syncRecordingToastState(
    nextBootstrap: AppBootstrap,
    options?: { notify?: boolean },
  ) {
    const previous = recordingToastStateRef.current;
    const next = {
      phase: nextBootstrap.shell.phase,
      transitionCount: nextBootstrap.shell.transitionCount,
    };

    recordingToastStateRef.current = next;

    if (
      !options?.notify ||
      (previous.phase === next.phase &&
        previous.transitionCount === next.transitionCount)
    ) {
      return;
    }

    const previousPhase = previous.phase;
    const nextPhase = next.phase;
    const recordingName =
      nextBootstrap.shell.currentRecordingName?.trim() || "Recording";

    if (nextPhase === "recording" && previousPhase !== "recording") {
      toast.success("Recording started", {
        description: recordingName,
        duration: 2500,
      });
      return;
    }

    if (nextPhase === "saving" && previousPhase === "recording") {
      toast("Recording stopped", {
        description: "Saving and processing the audio.",
        duration: 2500,
      });
      return;
    }

    if (
      nextPhase === "idle" &&
      (previousPhase === "saving" ||
        previousPhase === "transcribing" ||
        previousPhase === "recording")
    ) {
      toast.success("Recording saved", {
        description: nextBootstrap.shell.statusText,
        duration: 3500,
      });
      return;
    }

    if (nextPhase === "error" && previousPhase !== "error") {
      toast.error("Recording failed", {
        description: nextBootstrap.shell.statusText,
        duration: 5000,
      });
    }
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
        syncRecordingToastState(nextBootstrap);
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
      syncRecordingToastState(event.payload, { notify: true });
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

  useEffect(() => {
    let cancelled = false;

    async function refreshWhenAnkiIsAvailable() {
      if (cancelled || ankiAutoRefreshInFlightRef.current) {
        return;
      }

      ankiAutoRefreshInFlightRef.current = true;
      try {
        await refreshAnkiCatalog(undefined, {
          skipPersist: true,
          silent: true,
          suppressErrors: true,
        });
      } finally {
        ankiAutoRefreshInFlightRef.current = false;
      }
    }

    void refreshWhenAnkiIsAvailable();
    const interval = window.setInterval(refreshWhenAnkiIsAvailable, 10000);

    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [settingsDraft.anki.noteType]);

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
  const showBusyOverlay =
    isSaving ||
    isTranscribing ||
    busyAction === "transcribeRecording" ||
    busyAction === "translateRecording" ||
    busyAction === "pushAnki" ||
    busyAction === "addFurigana" ||
    busyAction === "deleteRecording" ||
    busyAction === "loadAnki" ||
    busyAction === "convertMp3";
  const busyOverlayLabel = isTranscribing || busyAction === "transcribeRecording"
    ? "Transcribing saved audio..."
    : busyAction === "translateRecording"
      ? "Translating transcript..."
      : busyAction === "pushAnki"
        ? "Pushing cards to Anki..."
        : busyAction === "addFurigana"
          ? "Adding furigana to Anki cards..."
          : busyAction === "deleteRecording"
            ? "Deleting selected recordings..."
            : busyAction === "convertMp3"
              ? "Converting recordings to MP3..."
              : busyAction === "loadAnki"
                ? "Loading Anki card data..."
                : isSaving
                ? "Finalizing the recording..."
                : "Working...";
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
  const {
    availableAnkiDecks,
    configuredAnkiDeckLabel,
    configuredDeckMenuOptions,
    convertibleRecordings,
    clearRecordingSelection,
    displayedAnkiCatalog,
    openRecordingMenuPath,
    pushableRecordings,
    recordingFilter,
    recordingFilterTabs,
    recordingPushedToCurrentAnkiDeck,
    recordingPushedToDeck,
    selectedConvertibleRecordings,
    selectedFuriganaRecordings,
    selectedPushableRecordings,
    selectedRecordings,
    selectedRecordingsPushableToDeck,
    selectedTranscribedRecordings,
    selectedUntranslatedRecordings,
    selectedUntranscribedRecordings,
    setOpenRecordingMenuPath,
    setRecordingFilter,
    toggleRecordingSelection,
    untranslatedRecordings,
    untranscribedRecordings,
    visibleRecordings,
    visibleSelectedPaths,
  } = useRecordingLibrary({
    ankiCatalog,
    ankiSettings: settingsDraft.anki,
    recentRecordings: bootstrap.recentRecordings,
  });
  const workflowPages = createWorkflowPages(bootstrap.recentRecordings.length);
  const setupPages = createSetupPages({
    whisperStatus: whisperStatusLabel(bootstrap.whisperDetection.status),
    runtimeVersion: activeRuntimeVersion,
    modelLabel: selectedModel.label,
    ffmpegReady: bootstrap.ffmpegDetection.status === "ready",
    ankiReady: displayedAnkiCatalog.status === "ready",
  });
  const currentPageLabel = activePageLabel(activePage, [
    ...workflowPages,
    ...setupPages,
  ]);

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
      setBootstrap((current) => ({
        ...current,
        shell: {
          ...current.shell,
          phase: "recording",
          statusText: "Starting system audio capture...",
          startedAtMs: Date.now(),
          currentRecordingName: "Starting recording",
          lastOutputPath: null,
          lastTranscriptPath: null,
          transitionCount: current.shell.transitionCount + 1,
        },
      }));
      await persistSettingsIfNeeded();
      const nextBootstrap = await invoke<AppBootstrap>("start_recording", {
        requestedName: null,
      });
      applyBootstrap(nextBootstrap);
    } catch (error) {
      try {
        applyBootstrap(await invoke<AppBootstrap>("get_app_bootstrap"));
      } catch {
        // Keep the original startup error visible if recovery snapshot loading fails.
      }
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
      setActivePage("runtime");
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
      setActivePage("storage");
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
      setActivePage("model");
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

  async function refreshAnkiCatalog(
    noteType?: string,
    options?: { silent?: boolean; skipPersist?: boolean; suppressErrors?: boolean },
  ) {
    try {
      if (!options?.silent) {
        setBusyAction("loadAnki");
      }
      if (!options?.skipPersist) {
        await persistSettingsIfNeeded();
      }
      const catalog = await invoke<AnkiCatalog>("load_anki_catalog", {
        noteType: (noteType ?? settingsDraft.anki.noteType) || null,
      });
      setAnkiCatalog(catalog);
    } catch (error) {
      if (!options?.suppressErrors) {
        setLoadError(
          error instanceof Error
            ? error.message
            : "The Anki catalog could not be loaded.",
        );
      }
    } finally {
      if (!options?.silent) {
        setBusyAction(null);
      }
    }
  }

  const {
    addFuriganaToAnki,
    convertRecordingsToMp3,
    deleteRecording,
    deleteRecordings,
    playRecording,
    pushRecordingsToAnki,
    transcribeRecordings,
    translateRecordings,
  } = useRecordingActions({
    applyBootstrap,
    persistSettingsIfNeeded,
    setBusyAction,
    setLoadError,
    setRecordingActionMessage,
    showSuccess,
    showWarning,
  });

  function updateAnkiField(field: keyof AnkiFieldMapping, value: string) {
    updateSettings({
      anki: {
        fields: {
          [field]: value,
        },
      },
    });
  }

  return (
    <main className="app-shell">
      <TooltipPrimitive.Provider delayDuration={180}>
        <Toaster
          position="top-right"
          richColors
          closeButton
          toastOptions={{
            className: "app-toast",
          }}
        />

      {loadError ? (
        <section className="banner banner-error">{loadError}</section>
      ) : null}

      {showBusyOverlay ? (
        <BusyOverlay
          label={busyOverlayLabel}
          statusText={bootstrap.shell.statusText}
        />
      ) : null}

      <section className="workspace">
        <section className="app-layout">
          <PageSidebar
            activePage={activePage}
            activePageLabel={currentPageLabel}
            workflowPages={workflowPages}
            setupPages={setupPages}
            onPageSelect={setActivePage}
          />

          <section className="content-column">
          {activePage === "recorder" ? (
            <RecorderPage
              elapsedMs={elapsedRecordingMs}
              phase={bootstrap.shell.phase}
              statusText={bootstrap.shell.statusText}
              hotkeyTooltip={hotkeyTooltip}
              recorderBusy={recorderBusy}
              isRecording={isRecording}
              stopBusy={busyAction === "stop"}
              anyBusy={busyAction !== null}
              onStartRecording={() => void startRecording()}
              onStopRecording={() => void stopRecording()}
              onHideToTray={() => void hideToTray()}
            />
          ) : null}

          {activePage === "recordings" ? (
            <SavedRecordingsPage
              recordingActionMessage={recordingActionMessage}
              recentRecordings={bootstrap.recentRecordings}
              visibleRecordings={visibleRecordings}
              recordingFilter={recordingFilter}
              recordingFilterTabs={recordingFilterTabs}
              selectedRecordings={selectedRecordings}
              visibleSelectedPaths={visibleSelectedPaths}
              configuredAnkiDeckLabel={configuredAnkiDeckLabel}
              configuredDeckMenuOptions={configuredDeckMenuOptions}
              currentDeckName={settingsDraft.anki.deckName}
              availableAnkiDecks={availableAnkiDecks}
              busyAction={busyAction}
              allowMp3Conversion={settingsDraft.features.allowMp3Conversion}
              expressionFieldMapped={Boolean(settingsDraft.anki.fields.transcription)}
              selectedUntranscribedRecordings={selectedUntranscribedRecordings}
              selectedPushableRecordings={selectedPushableRecordings}
              selectedTranscribedRecordings={selectedTranscribedRecordings}
              selectedFuriganaRecordings={selectedFuriganaRecordings}
              selectedUntranslatedRecordings={selectedUntranslatedRecordings}
              selectedConvertibleRecordings={selectedConvertibleRecordings}
              untranscribedRecordings={untranscribedRecordings}
              pushableRecordings={pushableRecordings}
              untranslatedRecordings={untranslatedRecordings}
              convertibleRecordings={convertibleRecordings}
              openRecordingMenuPath={openRecordingMenuPath}
              selectedRecordingsPushableToDeck={selectedRecordingsPushableToDeck}
              recordingPushedToDeck={recordingPushedToDeck}
              recordingPushedToCurrentAnkiDeck={recordingPushedToCurrentAnkiDeck}
              onFilterChange={setRecordingFilter}
              onDefaultDeckChange={(deck) =>
                updateSettings({
                  anki: {
                    deckName: deck,
                  },
                })
              }
              onRefreshAnki={() => void refreshAnkiCatalog()}
              onToggleSelection={toggleRecordingSelection}
              onClearSelection={clearRecordingSelection}
              onOpenRecordingMenuChange={setOpenRecordingMenuPath}
              onPlay={playRecording}
              onTranscribe={transcribeRecordings}
              onPushToAnki={pushRecordingsToAnki}
              onAddFurigana={addFuriganaToAnki}
              onTranslate={translateRecordings}
              onConvertToMp3={convertRecordingsToMp3}
              onDeleteRecording={deleteRecording}
              onDeleteRecordings={deleteRecordings}
            />
          ) : null}

          <SettingsPages
            activePage={activePage}
            bootstrap={bootstrap}
            settingsDraft={settingsDraft}
            autosaveState={autosaveState}
            autosaveMessage={autosaveMessage}
            busyAction={busyAction}
            displayedAnkiCatalog={displayedAnkiCatalog}
            activeRuntimeVersion={activeRuntimeVersion}
            installedRuntimeVersions={installedRuntimeVersions}
            manualRuntimeOverride={manualRuntimeOverride}
            runtimeUpdateResult={runtimeUpdateResult}
            runtimeUpdateVersion={runtimeUpdateVersion}
            modelUpdateResult={modelUpdateResult}
            runtimeInstalled={runtimeInstalled}
            modelInstalled={modelInstalled}
            resolvedCliPath={resolvedCliPath}
            resolvedModelPath={resolvedModelPath}
            downloadIsActive={downloadIsActive}
            onUpdateSettings={updateSettings}
            onBrowseDirectory={browseForDirectory}
            onBrowseFile={browseForFile}
            onCheckRuntimeUpdate={checkRuntimeUpdate}
            onDownloadRuntimeVersion={downloadRuntimeVersion}
            onDownloadRecommendedRuntime={downloadRecommendedRuntime}
            onCheckModelUpdate={checkModelUpdate}
            onDownloadRecommendedModel={downloadRecommendedModel}
            onDownloadRecommendedFfmpeg={downloadRecommendedFfmpeg}
            onToggleDownloadPause={toggleDownloadPause}
            onCancelDownload={cancelDownload}
            onRefreshAnkiCatalog={refreshAnkiCatalog}
            onUpdateAnkiField={updateAnkiField}
          />
          </section>
        </section>
      </section>
      </TooltipPrimitive.Provider>
    </main>
  );
}

export default App;
