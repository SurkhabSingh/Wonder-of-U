import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import * as TooltipPrimitive from "@radix-ui/react-tooltip";
import { Toaster, toast } from "sonner";
import { PageSidebar } from "./components/layout/PageSidebar";
import { RecorderPage } from "./components/recorder/RecorderPage";
import { SavedRecordingsPage } from "./components/recordings/SavedRecordingsPage";
import { AnkiFieldSelect } from "./components/settings/AnkiFieldSelect";
import { DownloadProgressCard } from "./components/settings/DownloadProgressCard";
import { BusyOverlay } from "./components/ui/BusyOverlay";
import { ThemedSelect } from "./components/ui/ThemedSelect";
import { TooltipBadge } from "./components/ui/Tooltip";
import { UpdateResultCard } from "./components/ui/UpdateResultCard";
import {
  APP_SNAPSHOT_EVENT,
  DEFAULT_ANKI_CATALOG,
  DEFAULT_BOOTSTRAP,
  LANGUAGE_OPTIONS,
  MODEL_OPTIONS,
  RECOMMENDED_RUNTIME_VERSION,
} from "./constants";
import { fileNameFromPath } from "./lib/format";
import {
  normalizeSelection,
  pathHasExtension,
  recordingSupportsFurigana,
  whisperStatusLabel,
} from "./lib/helpers";
import {
  activePageLabel,
  createRecordingFilterTabs,
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
  RecentRecording,
  RecordingBatchResult,
  RecordingFilter,
  ThemePreference,
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
  const [selectedRecordings, setSelectedRecordings] = useState<string[]>([]);
  const [recordingFilter, setRecordingFilter] = useState<RecordingFilter>("all");
  const [openRecordingMenuPath, setOpenRecordingMenuPath] = useState<string | null>(
    null,
  );
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

  function mergeSavedAnkiSettingsIntoCatalog(catalog: AnkiCatalog): AnkiCatalog {
    const savedFields = Object.values(settingsDraft.anki.fields).filter(Boolean);
    return {
      ...catalog,
      decks: Array.from(
        new Set([
          ...(settingsDraft.anki.deckName ? [settingsDraft.anki.deckName] : []),
          ...catalog.decks,
        ]),
      ),
      noteTypes: Array.from(
        new Set([
          ...(settingsDraft.anki.noteType ? [settingsDraft.anki.noteType] : []),
          ...catalog.noteTypes,
        ]),
      ),
      fields: Array.from(new Set([...savedFields, ...catalog.fields])),
      message:
        catalog.status === "idle" &&
        (settingsDraft.anki.deckName || settingsDraft.anki.noteType)
          ? "Using your saved Anki mapping. Refresh only if you changed decks, note types, or fields in Anki."
          : catalog.message,
    };
  }

  function formatBatchToastMessage(
    action: "transcribe" | "translate" | "delete" | "anki" | "furigana" | "convert",
    result: RecordingBatchResult,
  ): string {
    const successCount = result.items.filter((item) => item.status === "success").length;
    const skippedCount = result.items.filter((item) => item.status === "skipped").length;
    const failedItems = result.items.filter((item) => item.status === "failed");
    const failedCount = failedItems.length;
    const firstFailure = failedItems[0]?.message;
    const furiganaSkippedCount = result.items.filter((item) =>
      item.message.toLowerCase().includes("furigana was skipped"),
    ).length;

    if (action === "anki") {
      if (failedCount > 0 && successCount === 0) {
        return firstFailure
          ? `No cards were pushed to Anki. ${firstFailure}`
          : "No cards were pushed to Anki.";
      }
      if (failedCount > 0) {
        return `${successCount} card${successCount === 1 ? "" : "s"} pushed to Anki. ${failedCount} failed: ${firstFailure ?? "check the saved recordings list."}`;
      }
      if (successCount === 0 && skippedCount > 0) {
        return `${skippedCount} card${skippedCount === 1 ? " is" : "s are"} already in the selected Anki deck.`;
      }
      const baseMessage = `${successCount} card${successCount === 1 ? "" : "s"} pushed to Anki.`;
      return furiganaSkippedCount > 0
        ? `${baseMessage} Furigana was skipped for ${furiganaSkippedCount} because the Anki Lookup add-on was unavailable.`
        : baseMessage;
    }

    if (action === "furigana") {
      if (failedCount > 0 && successCount === 0) {
        return firstFailure
          ? `No Anki cards were updated with furigana. ${firstFailure}`
          : "No Anki cards were updated with furigana.";
      }
      if (failedCount > 0) {
        return `${successCount} Anki card${successCount === 1 ? "" : "s"} updated with furigana. ${failedCount} failed: ${firstFailure ?? "check the saved recordings list."}`;
      }
      return `${successCount} Anki card${successCount === 1 ? "" : "s"} updated with furigana.`;
    }

    if (action === "convert") {
      if (failedCount > 0 && successCount === 0) {
        return firstFailure ?? "No recordings were converted to MP3.";
      }
      if (failedCount > 0) {
        return `${successCount} recording${successCount === 1 ? "" : "s"} converted to MP3. ${failedCount} failed.`;
      }
      if (successCount === 0 && skippedCount > 0) {
        return `${skippedCount} recording${skippedCount === 1 ? " was" : "s were"} skipped. Only transcribed WAV recordings can be converted.`;
      }
      return `${successCount} recording${successCount === 1 ? "" : "s"} converted to MP3.`;
    }

    if (failedCount > 0 && successCount === 0) {
      return firstFailure ?? result.message;
    }

    return result.message;
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
  const recordingPushedToDeck = (recording: RecentRecording, deckName: string) =>
    recording.ankiNoteId !== null &&
    recording.ankiDeckName !== null &&
    recording.ankiDeckName === deckName;
  const recordingPushableToDeck = (recording: RecentRecording, deckName: string) =>
    deckName.trim().length > 0 &&
    Boolean(recording.transcriptPath) &&
    !recording.audioDeleted &&
    !recordingPushedToDeck(recording, deckName);
  const recordingPushedToCurrentAnkiDeck = (recording: RecentRecording) =>
    recordingPushedToDeck(recording, settingsDraft.anki.deckName);
  const pushableRecordings = transcribedRecordings.filter(
    (recording) =>
      !recording.audioDeleted && !recordingPushedToCurrentAnkiDeck(recording),
  );
  const untranslatedRecordings = transcribedRecordings.filter(
    (recording) => recording.translationPath === null,
  );
  const completeRecordings = bootstrap.recentRecordings.filter(
    (recording) =>
      Boolean(recording.transcriptPath) &&
      recordingPushedToCurrentAnkiDeck(recording) &&
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
  const useBatchActionsOnly = visibleSelectedPaths.length > 1;
  const selectedTranscribedRecordings = visibleSelectedRecordings.filter(
    (recording) => recording.transcriptPath,
  );
  const selectedPushableRecordings = selectedTranscribedRecordings.filter(
    (recording) => recordingPushableToDeck(recording, settingsDraft.anki.deckName),
  );
  const selectedUntranscribedRecordings = visibleSelectedRecordings.filter(
    (recording) => !recording.transcriptPath,
  );
  const selectedUntranslatedRecordings = selectedTranscribedRecordings.filter(
    (recording) => recording.translationPath === null,
  );
  const selectedFuriganaRecordings = selectedTranscribedRecordings.filter(
    (recording) =>
      recording.ankiNoteId !== null && recordingSupportsFurigana(recording),
  );
  const convertibleRecordings = bootstrap.recentRecordings.filter(
    (recording) =>
      !recording.audioDeleted &&
      recording.transcriptPath !== null &&
      pathHasExtension(recording.filePath, "wav"),
  );
  const selectedConvertibleRecordings = visibleSelectedRecordings.filter(
    (recording) =>
      !recording.audioDeleted &&
      recording.transcriptPath !== null &&
      pathHasExtension(recording.filePath, "wav"),
  );
  const recordingFilterTabs = createRecordingFilterTabs({
    allCount: bootstrap.recentRecordings.length,
    untranscribedCount: untranscribedRecordings.length,
    pushableCount: pushableRecordings.length,
    untranslatedCount: untranslatedRecordings.length,
    completeCount: completeRecordings.length,
  });
  const displayedAnkiCatalog = mergeSavedAnkiSettingsIntoCatalog(ankiCatalog);
  const configuredAnkiDeckLabel =
    settingsDraft.anki.deckName.trim() || "No deck selected";
  const availableAnkiDecks = displayedAnkiCatalog.decks.filter(
    (deck) => deck.trim().length > 0,
  );
  const configuredDeckMenuOptions = availableAnkiDecks.length > 0
    ? availableAnkiDecks
    : settingsDraft.anki.deckName
      ? [settingsDraft.anki.deckName]
      : [];
  const selectedRecordingsPushableToDeck = (deckName: string) =>
    selectedTranscribedRecordings.filter((recording) =>
      recordingPushableToDeck(recording, deckName),
    );
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

  useEffect(() => {
    if (useBatchActionsOnly) {
      setOpenRecordingMenuPath(null);
    }
  }, [useBatchActionsOnly]);

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
    const confirmed = window.confirm(
      "Delete this saved recording from Wonder of U? This removes the local audio, transcript, and translation files from this machine. Existing Anki cards are not affected.",
    );
    if (!confirmed) {
      return;
    }

    try {
      setBusyAction("deleteRecording");
      const nextBootstrap = await invoke<AppBootstrap>("delete_recording", {
        filePath,
      });
      applyBootstrap(nextBootstrap);
      setRecordingActionMessage("Recording deleted.");
      showSuccess("Recording deleted.");
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
      } from Wonder of U? This removes local audio, transcript, and translation files from this machine. Existing Anki cards are not affected.`,
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
      showSuccess(result.message);
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

  async function pushRecordingsToAnki(filePaths: string[], deckName?: string) {
    try {
      setBusyAction("pushAnki");
      await persistSettingsIfNeeded();
      const targetDeck = deckName?.trim();
      const result = await invoke<RecordingBatchResult>(
        targetDeck ? "push_recordings_to_anki_deck" : "push_recordings_to_anki",
        targetDeck ? { filePaths, deckName: targetDeck } : { filePaths },
      );
      applyBootstrap(result.bootstrap);
      const message = formatBatchToastMessage("anki", result);
      setRecordingActionMessage(message);
      if (
        result.status === "unavailable" ||
        result.status === "partial" ||
        message.toLowerCase().includes("anki is currently offline") ||
        message.toLowerCase().includes("no cards were pushed") ||
        message.toLowerCase().includes("furigana was skipped")
      ) {
        showWarning(message);
      } else {
        showSuccess(message);
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

  async function addFuriganaToAnki(filePaths: string[]) {
    try {
      setBusyAction("addFurigana");
      await persistSettingsIfNeeded();
      const result = await invoke<RecordingBatchResult>("add_furigana_to_anki", {
        filePaths,
      });
      applyBootstrap(result.bootstrap);
      const message = formatBatchToastMessage("furigana", result);
      setRecordingActionMessage(message);
      if (result.status === "unavailable" || result.status === "partial") {
        showWarning(message);
      } else {
        showSuccess(message);
      }
    } catch (error) {
      const message =
        error instanceof Error
          ? error.message
          : "Furigana could not be added to Anki cards.";
      if (
        message.toLowerCase().includes("anki") ||
        message.toLowerCase().includes("furigana")
      ) {
        showWarning(message);
      }
      setLoadError(message);
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
      const message = formatBatchToastMessage("transcribe", result);
      setRecordingActionMessage(message);
      showSuccess(message);
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
      const message = formatBatchToastMessage("translate", result);
      setRecordingActionMessage(message);
      showSuccess(message);
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

  async function convertRecordingsToMp3(filePaths: string[]) {
    const confirmed = window.confirm(
      `Convert ${filePaths.length} recording${
        filePaths.length === 1 ? "" : "s"
      } to MP3? Wonder of U will keep the transcript/history, create MP3 files, and remove the original local WAV files after conversion succeeds. Existing Anki cards are not affected.`,
    );
    if (!confirmed) {
      return;
    }

    try {
      setBusyAction("convertMp3");
      const result = await invoke<RecordingBatchResult>("convert_recordings_to_mp3", {
        filePaths,
      });
      applyBootstrap(result.bootstrap);
      const message = formatBatchToastMessage("convert", result);
      setRecordingActionMessage(message);
      if (result.status === "partial") {
        showWarning(message);
      } else {
        showSuccess(message);
      }
    } catch (error) {
      const message =
        error instanceof Error
          ? error.message
          : "The selected recordings could not be converted to MP3.";
      showWarning(message);
      setLoadError(message);
    } finally {
      setBusyAction(null);
    }
  }

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

          {activePage !== "recorder" && activePage !== "recordings" ? (
            <div className="settings-scroll settings-page-single">
              <div className="settings-overview-grid">
                <article
                  className="panel settings-card"
                  hidden={activePage !== "preferences"}
                >
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
                    <ThemedSelect
                      value={settingsDraft.theme}
                      options={[
                        { value: "system", label: "Use system setting" },
                        { value: "light", label: "Light" },
                        { value: "dark", label: "Dark" },
                      ]}
                      placeholder="Appearance"
                      onChange={(nextValue) =>
                        updateSettings({
                          theme: nextValue as ThemePreference,
                        })
                      }
                    />
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
                        checked={
                          settingsDraft.features.deleteLocalAudioAfterAnkiPush
                        }
                        onChange={(event) => {
                          const enabled = event.currentTarget.checked;
                          if (enabled) {
                            const confirmed = window.confirm(
                              "Enable local audio cleanup after Anki push? After Anki successfully copies the audio into its media folder, Wonder of U will delete the local audio file from this machine. The transcript and history stay in Wonder of U, and existing Anki cards are not affected.",
                            );
                            if (!confirmed) {
                              return;
                            }
                          }
                          updateSettings({
                            features: {
                              deleteLocalAudioAfterAnkiPush: enabled,
                            },
                          });
                        }}
                      />
                      <span>Delete local audio after Anki push</span>
                    </label>

                    <label className="toggle">
                      <input
                        type="checkbox"
                        checked={settingsDraft.features.allowMp3Conversion}
                        onChange={(event) =>
                          updateSettings({
                            features: {
                              allowMp3Conversion: event.currentTarget.checked,
                            },
                          })
                        }
                      />
                      <span>Allow manual MP3 conversion</span>
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

                <article
                  className="panel settings-card"
                  hidden={activePage !== "whisper"}
                >
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
                <article
                  className="panel settings-card"
                  hidden={activePage !== "runtime"}
                >
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
                      <ThemedSelect
                        value={activeRuntimeVersion}
                        options={installedRuntimeVersions.map((version) => ({
                          value: version,
                          label: version,
                        }))}
                        placeholder="Active runtime"
                        onChange={(nextValue) =>
                          updateSettings({
                            whisper: {
                              runtimeVersion: nextValue,
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
                      />
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
                        <UpdateResultCard result={runtimeUpdateResult} />
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
                    <DownloadProgressCard
                      snapshot={bootstrap.modelDownload}
                      kind="runtime"
                      downloadIsActive={downloadIsActive}
                      onTogglePause={() => void toggleDownloadPause()}
                      onCancel={() => void cancelDownload()}
                    />
                  </div>
                </article>

                <article
                  className="panel settings-card"
                  hidden={activePage !== "model"}
                >
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
                    <ThemedSelect
                      value={settingsDraft.whisper.modelChoice}
                      options={MODEL_OPTIONS.map((option) => ({
                        value: option.id,
                        label: option.label,
                      }))}
                      placeholder="Managed model"
                      onChange={(nextValue) =>
                        updateSettings({
                          whisper: {
                            modelChoice: nextValue,
                          },
                        })
                      }
                    />
                  </label>

                  <label className="field">
                    <span>Language</span>
                    <ThemedSelect
                      value={selectedLanguageCode}
                      options={[
                        ...(!selectedLanguageKnown
                          ? [
                              {
                                value: selectedLanguageCode,
                                label: `Custom (${selectedLanguageCode})`,
                              },
                            ]
                          : []),
                        ...LANGUAGE_OPTIONS.map((option) => ({
                          value: option.code,
                          label: `${option.label} (${option.code})`,
                        })),
                      ]}
                      placeholder="Language"
                      onChange={(nextValue) =>
                        updateSettings({
                          whisper: {
                            language: nextValue,
                          },
                        })
                      }
                    />
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
                      <UpdateResultCard result={modelUpdateResult} />
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
                  <DownloadProgressCard
                    snapshot={bootstrap.modelDownload}
                    kind="model"
                    downloadIsActive={downloadIsActive}
                    onTogglePause={() => void toggleDownloadPause()}
                    onCancel={() => void cancelDownload()}
                  />
                </div>
                </article>

                <article
                  className="panel settings-card settings-card-wide"
                  hidden={activePage !== "storage"}
                >
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
                      Wonder of U keeps WAV audio for transcription because that is
                      the safest input path for Whisper. After a transcript exists,
                      you can convert individual recordings, selected recordings, or
                      all available WAV recordings to MP3 from Saved Recordings.
                      If a card was already pushed to Anki, converting the local
                      file later will not break that existing Anki card because
                      Anki keeps its own copied media file.
                      The Convert to MP3 action stays hidden until you enable
                      manual MP3 conversion in App Preferences.
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
                  <DownloadProgressCard
                    snapshot={bootstrap.modelDownload}
                    kind="ffmpeg"
                    downloadIsActive={downloadIsActive}
                    onTogglePause={() => void toggleDownloadPause()}
                    onCancel={() => void cancelDownload()}
                  />
                </article>
              </div>

              <article
                className="panel anki-panel settings-card settings-card-wide"
                hidden={activePage !== "anki"}
              >
                <header className="panel-header">
                  <div>
                    <p className="panel-kicker">Anki</p>
                    <h2>Card Mapping</h2>
                  </div>
                  <div className="panel-actions">
                    <TooltipBadge
                      label={displayedAnkiCatalog.status === "ready" ? "Ready" : "Saved"}
                      description={displayedAnkiCatalog.message}
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
                    displayedAnkiCatalog.status === "ready"
                      ? "current"
                      : displayedAnkiCatalog.status === "offline"
                        ? "error"
                        : ""
                  }`}
                >
                  <strong>{displayedAnkiCatalog.message}</strong>
                  {displayedAnkiCatalog.version !== null ? (
                    <p className="microcopy">
                      AnkiConnect version {displayedAnkiCatalog.version}
                    </p>
                  ) : null}
                </div>

                <div className="settings-grid anki-grid">
                  <label className="field">
                    <span className="field-label-with-help">
                      <span>Deck</span>
                      <TooltipBadge
                        label="?"
                        description="Cards are created in this Anki deck when you use the default Push action. Push to another deck overrides this only for that action."
                      />
                    </span>
                    <ThemedSelect
                      value={settingsDraft.anki.deckName}
                      options={[
                        { value: "", label: "Choose deck" },
                        ...(settingsDraft.anki.deckName &&
                        !displayedAnkiCatalog.decks.includes(settingsDraft.anki.deckName)
                          ? [
                              {
                                value: settingsDraft.anki.deckName,
                                label: settingsDraft.anki.deckName,
                              },
                            ]
                          : []),
                        ...displayedAnkiCatalog.decks.map((deck) => ({
                          value: deck,
                          label: deck,
                        })),
                      ]}
                      placeholder="Choose deck"
                      onChange={(nextValue) =>
                        updateSettings({
                          anki: {
                            deckName: nextValue,
                          },
                        })
                      }
                    />
                  </label>

                  <label className="field">
                    <span className="field-label-with-help">
                      <span>Note type</span>
                      <TooltipBadge
                        label="?"
                        description="This controls which Anki fields are available below. If you change the note type, the field mapping is reset because each note type has different fields."
                      />
                    </span>
                    <ThemedSelect
                      value={settingsDraft.anki.noteType}
                      options={[
                        { value: "", label: "Choose note type" },
                        ...(settingsDraft.anki.noteType &&
                        !displayedAnkiCatalog.noteTypes.includes(settingsDraft.anki.noteType)
                          ? [
                              {
                                value: settingsDraft.anki.noteType,
                                label: settingsDraft.anki.noteType,
                              },
                            ]
                          : []),
                        ...displayedAnkiCatalog.noteTypes.map((noteType) => ({
                          value: noteType,
                          label: noteType,
                        })),
                      ]}
                      placeholder="Choose note type"
                      onChange={(noteType) => {
                        updateSettings({
                          anki: {
                            noteType,
                            fields: {
                              transcription: "",
                              furigana: "",
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
                    />
                  </label>

                  <AnkiFieldSelect
                    field="transcription"
                    label="Expression / transcript field"
                    description="Receives the transcript during push. When furigana is enabled or added later, this same field is replaced with hover-only ruby HTML, like a Yomitan expression field."
                    currentValue={settingsDraft.anki.fields.transcription}
                    fieldOptions={displayedAnkiCatalog.fields}
                    onChange={updateAnkiField}
                  />
                  <AnkiFieldSelect
                    field="audio"
                    label="Replay audio field"
                    description="Receives the [sound:...] tag. The replay icon only appears on card sides that render this field. If it disappears after revealing the answer, the Back template must include the front side or this audio field."
                    currentValue={settingsDraft.anki.fields.audio}
                    fieldOptions={displayedAnkiCatalog.fields}
                    onChange={updateAnkiField}
                  />
                  <AnkiFieldSelect
                    field="translation"
                    label="Translation field"
                    description="Optional translated text. Leave unmapped if you do not want translations written to Anki."
                    currentValue={settingsDraft.anki.fields.translation}
                    fieldOptions={displayedAnkiCatalog.fields}
                    onChange={updateAnkiField}
                  />
                  <AnkiFieldSelect
                    field="sourcePath"
                    label="Source path field"
                    description="Optional local audio path for your own tracking. This is not required for playback after Anki copies the media."
                    currentValue={settingsDraft.anki.fields.sourcePath}
                    fieldOptions={displayedAnkiCatalog.fields}
                    onChange={updateAnkiField}
                  />
                  <AnkiFieldSelect
                    field="createdAt"
                    label="Created-at field"
                    description="Optional recording timestamp in milliseconds. Leave unmapped unless your note type has a tracking field for it."
                    currentValue={settingsDraft.anki.fields.createdAt}
                    fieldOptions={displayedAnkiCatalog.fields}
                    onChange={updateAnkiField}
                  />
                </div>

                <div className="update-card">
                  <label className="toggle inline-toggle">
                    <input
                      type="checkbox"
                      checked={
                        settingsDraft.features.autoAddFuriganaAfterAnkiPush
                      }
                      onChange={(event) =>
                        updateSettings({
                          features: {
                            autoAddFuriganaAfterAnkiPush:
                              event.currentTarget.checked,
                          },
                        })
                      }
                    />
                    <span>
                      Automatically add furigana when pushing Japanese cards
                    </span>
                  </label>
                  <p className="microcopy">
                    Requires the Wonder of U Anki add-on to be running. If the
                    add-on is unavailable, Wonder of U still pushes the card and
                    warns that furigana was skipped. Furigana is written onto
                    the expression/transcript field itself.
                  </p>
                </div>

                <div className="update-card">
                  <strong>Recommended mapping: Expression / transcript -&gt; Expression or Back, Replay audio -&gt; Audio or Front.</strong>
                  <p className="microcopy">
                    Furigana is applied directly to the expression/transcript
                    field, not a separate field. The Anki replay icon only
                    shows if the audio field is visible in the current card side
                    template.
                  </p>
                </div>
              </article>
            </div>
          ) : null}
        </section>
        </section>
      </section>
      </TooltipPrimitive.Provider>
    </main>
  );
}

export default App;
