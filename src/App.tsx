import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import * as TooltipPrimitive from "@radix-ui/react-tooltip";
import { Toaster, toast } from "sonner";
import { PageSidebar } from "./components/layout/PageSidebar";
import { RecorderPage } from "./components/recorder/RecorderPage";
import { SavedRecordingsPage } from "./components/recordings/SavedRecordingsPage";
import { SettingsPages } from "./components/settings/SettingsPages";
import { BusyOverlay } from "./components/ui/BusyOverlay";
import {
  DEFAULT_ANKI_CATALOG,
  MODEL_OPTIONS,
  RECOMMENDED_RUNTIME_VERSION,
} from "./constants";
import { useAppBootstrap } from "./hooks/useAppBootstrap";
import { useRecordingActions } from "./hooks/useRecordingActions";
import { useRecordingLibrary } from "./hooks/useRecordingLibrary";
import { useRecorderActions } from "./hooks/useRecorderActions";
import { useSetupActions } from "./hooks/useSetupActions";
import { whisperStatusLabel } from "./lib/helpers";
import {
  activePageLabel,
  createSetupPages,
  createWorkflowPages,
} from "./lib/navigation";
import type {
  AnkiCatalog,
  AppPage,
  BusyAction,
  WhisperAssetUpdateResult,
} from "./types";

function App() {
  const {
    applyBootstrap,
    autosaveMessage,
    autosaveState,
    bootstrap,
    loadError,
    persistSettingsIfNeeded,
    setBootstrap,
    setLoadError,
    settingsDraft,
    updateSettings,
  } = useAppBootstrap();
  const [systemTheme, setSystemTheme] = useState<"light" | "dark">(() =>
    window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light",
  );
  const [busyAction, setBusyAction] = useState<BusyAction>(null);
  const [clockMs, setClockMs] = useState(() => Date.now());
  const [activePage, setActivePage] = useState<AppPage>("recorder");
  const [runtimeUpdateResult, setRuntimeUpdateResult] =
    useState<WhisperAssetUpdateResult | null>(null);
  const [modelUpdateResult, setModelUpdateResult] =
    useState<WhisperAssetUpdateResult | null>(null);
  const [ankiCatalog, setAnkiCatalog] =
    useState<AnkiCatalog>(DEFAULT_ANKI_CATALOG);
  const [recordingActionMessage, setRecordingActionMessage] = useState("");
  const ankiAutoRefreshInFlightRef = useRef(false);
  const resolvedTheme =
    settingsDraft.theme === "system" ? systemTheme : settingsDraft.theme;

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

  function showWarning(message: string) {
    toast.warning(message, { duration: 5000 });
  }

  function showSuccess(message: string) {
    toast.success(message, { duration: 3500 });
  }

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

  const {
    browseForDirectory,
    browseForFile,
    cancelDownload,
    checkModelUpdate,
    checkRuntimeUpdate,
    downloadRecommendedFfmpeg,
    downloadRecommendedModel,
    downloadRecommendedRuntime,
    downloadRuntimeVersion,
    toggleDownloadPause,
    updateAnkiField,
  } = useSetupActions({
    applyBootstrap,
    persistSettingsIfNeeded,
    resolvedCliPath,
    resolvedModelPath,
    setActivePage,
    setBusyAction,
    setLoadError,
    setModelUpdateResult,
    setRuntimeUpdateResult,
    settingsDraft,
    updateSettings,
  });

  const { hideToTray, startRecording, stopRecording } = useRecorderActions({
    applyBootstrap,
    persistSettingsIfNeeded,
    setBootstrap,
    setBusyAction,
    setLoadError,
  });

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
