import { useCallback, useEffect, useState } from "react";
import * as TooltipPrimitive from "@radix-ui/react-tooltip";
import { Toaster, toast } from "sonner";
import { HomePage } from "./components/home/HomePage";
import { PageSidebar } from "./components/layout/PageSidebar";
import { SavedRecordingsPage } from "./components/recordings/SavedRecordingsPage";
import { SettingsPages } from "./components/settings/SettingsPages";
import { SetupChecklist } from "./components/settings/SetupChecklist";
import { TranscriptViewerPage } from "./components/transcripts/TranscriptViewerPage";
import { BusyOverlay } from "./components/ui/BusyOverlay";
import { useAnkiCatalog } from "./hooks/useAnkiCatalog";
import { useAppBootstrap } from "./hooks/useAppBootstrap";
import { useAppViewState } from "./hooks/useAppViewState";
import { useRecordingActions } from "./hooks/useRecordingActions";
import { useRecordingLibrary } from "./hooks/useRecordingLibrary";
import { useRecorderActions } from "./hooks/useRecorderActions";
import { useSetupActions } from "./hooks/useSetupActions";
import { useTranscriptionProgress } from "./hooks/useTranscriptionProgress";
import { useTranscriptionQueue } from "./hooks/useTranscriptionQueue";
import { useYoutubeQueue } from "./hooks/useYoutubeQueue";
import { fileNameFromPath } from "./lib/format";
import type {
  AppPage,
  BusyAction,
  SettingsSection,
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
  const [busyAction, setBusyAction] = useState<BusyAction>(null);
  const [activePage, setActivePage] = useState<AppPage>("home");
  const [settingsScrollTarget, setSettingsScrollTarget] =
    useState<SettingsSection | null>(null);
  const [viewingRecordingPath, setViewingRecordingPath] = useState<string | null>(
    null,
  );
  const [runtimeUpdateResult, setRuntimeUpdateResult] =
    useState<WhisperAssetUpdateResult | null>(null);
  const [modelUpdateResult, setModelUpdateResult] =
    useState<WhisperAssetUpdateResult | null>(null);
  const [ytdlpUpdateResult, setYtdlpUpdateResult] =
    useState<WhisperAssetUpdateResult | null>(null);
  const [recordingActionMessage, setRecordingActionMessage] = useState("");

  // The Library status microcopy is never cleared by its setters, so it lingers
  // on the page. Clear it ~6s after it becomes non-empty; the cleanup means each
  // new message resets the timer rather than stacking timeouts.
  useEffect(() => {
    if (!recordingActionMessage) return;
    const id = setTimeout(() => setRecordingActionMessage(""), 6000);
    return () => clearTimeout(id);
  }, [recordingActionMessage]);

  function showWarning(message: string) {
    toast.warning(message, { duration: 5000 });
  }

  function showSuccess(message: string) {
    toast.success(message, { duration: 3500 });
  }

  // Deep-link into the single Settings page and scroll a specific section into
  // view. Used by the Setup checklist rows and by post-download navigation.
  const openSettingsSection = useCallback((section: SettingsSection) => {
    setSettingsScrollTarget(section);
    setActivePage("settings");
  }, []);

  const clearSettingsScrollTarget = useCallback(() => {
    setSettingsScrollTarget(null);
  }, []);

  function openTranscriptViewer(filePath: string) {
    setViewingRecordingPath(filePath);
    setActivePage("transcript");
  }

  function closeTranscriptViewer() {
    setViewingRecordingPath(null);
    setActivePage("recordings");
  }

  const viewingRecording =
    viewingRecordingPath !== null
      ? bootstrap.recentRecordings.find(
          (recording) => recording.filePath === viewingRecordingPath,
        ) ?? null
      : null;

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

  const { ankiCatalog, refreshAnkiCatalog } = useAnkiCatalog({
    noteType: settingsDraft.anki.noteType,
    persistSettingsIfNeeded,
    setBusyAction,
    setLoadError,
    showSuccess,
    showWarning,
  });

  const runtimeUpdateVersion =
    runtimeUpdateResult?.status === "available"
      ? runtimeUpdateResult.latestVersion
      : null;
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
    recordingPage,
    recordingPageCount,
    recordingSearch,
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
    setRecordingPage,
    setRecordingSearch,
    toggleRecordingSelection,
    untranslatedRecordings,
    untranscribedRecordings,
    visibleRecordings,
    visibleSelectedPaths,
  } = useRecordingLibrary({
    ankiCatalog,
    ankiSettings: settingsDraft.anki,
    recentRecordings: bootstrap.recentRecordings,
    transcriptionLanguage: settingsDraft.whisper.language,
  });
  const {
    activeRuntimeVersion,
    busyOverlayLabel,
    downloadIsActive,
    elapsedRecordingMs,
    hotkeyTooltip,
    installedRuntimeVersions,
    isRecording,
    manualRuntimeOverride,
    modelInstalled,
    recorderBusy,
    resolvedCliPath,
    resolvedModelPath,
    runtimeInstalled,
    setupChecklist,
    setupEntry,
    setupSummary,
    showBusyOverlay,
    workflowPages,
  } = useAppViewState({
    activePage,
    bootstrap,
    busyAction,
    settingsDraft,
  });

  const {
    browseForDirectory,
    browseForFile,
    cancelDownload,
    checkModelUpdate,
    checkRuntimeUpdate,
    checkYtdlpUpdate,
    downloadRecommendedFfmpeg,
    downloadRecommendedModel,
    downloadRecommendedRuntime,
    downloadRecommendedYtdlp,
    downloadRuntimeVersion,
    toggleDownloadPause,
    updateAnkiField,
  } = useSetupActions({
    applyBootstrap,
    persistSettingsIfNeeded,
    resolvedCliPath,
    resolvedModelPath,
    openSettingsSection,
    setBusyAction,
    setLoadError,
    setModelUpdateResult,
    setRuntimeUpdateResult,
    setYtdlpUpdateResult,
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

  const {
    addFuriganaToAnki,
    convertRecordingsToMp3,
    deleteRecording,
    deleteRecordings,
    importMedia,
    importYoutube,
    mineSegment,
    pushRecordingsToAnki,
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

  // Sequential frontend queue over the single-URL YouTube import: a paste of
  // many links fetches one at a time on the shared download slot. Navigation to
  // the Library is deferred until the whole queue is done, not per URL.
  const youtubeQueue = useYoutubeQueue({
    importYoutube,
    onAllComplete: (landed) => {
      if (landed > 0) {
        setActivePage("recordings");
      }
    },
  });

  // Sequential frontend queue over the single-file transcribe command, so
  // transcription runs NON-blocking (the app stays usable while this queue shows
  // progress) instead of the old full-screen busy overlay. Each item applies its
  // returned bootstrap, so the Library refreshes as transcripts land.
  const transcriptionQueue = useTranscriptionQueue({
    applyBootstrap,
    persistSettingsIfNeeded,
  });

  // Adapt the shared `(filePaths, force)` action shape the Transcribe buttons use
  // to the queue's enqueue, stamping each row with the recording's display name.
  const enqueueTranscriptions = useCallback(
    (filePaths: string[], force = false) => {
      const files = filePaths.map((filePath) => {
        const recording = bootstrap.recentRecordings.find(
          (candidate) => candidate.filePath === filePath,
        );
        return {
          filePath,
          title: recording?.fileName ?? fileNameFromPath(filePath),
        };
      });
      transcriptionQueue.enqueue(files, force);
    },
    [bootstrap.recentRecordings, transcriptionQueue],
  );

  // Sentence mining needs a mapped expression field to write to and a reachable
  // Anki. `offline` is the only catalog status that definitively means "not
  // reachable"; idle/ready are treated as reachable (the click still reports
  // honestly if Anki turns out to be down).
  const expressionFieldMapped = Boolean(settingsDraft.anki.fields.transcription);
  const ankiReachable = displayedAnkiCatalog.status !== "offline";

  const transcriptionProgress = useTranscriptionProgress(
    bootstrap.shell.phase === "transcribing",
  );

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
          progress={transcriptionProgress}
        />
      ) : null}

      <section className="workspace">
        <section className="app-layout">
          <PageSidebar
            activePage={activePage}
            workflowPages={workflowPages}
            setupEntry={setupEntry}
            onPageSelect={setActivePage}
          />

          <section className="content-column">
          {activePage === "home" ? (
            <HomePage
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
              recentRecordings={bootstrap.recentRecordings}
              needsTranscriptCount={untranscribedRecordings.length}
              needsTranslationCount={untranslatedRecordings.length}
              readyForAnkiCount={pushableRecordings.length}
              transcriptionLanguage={settingsDraft.whisper.language}
              recordingPushedToCurrentAnkiDeck={recordingPushedToCurrentAnkiDeck}
              isImporting={busyAction === "importMedia"}
              onImportMedia={(paths) => {
                void importMedia(paths).then((result) => {
                  // Only jump to the Library when a file actually landed, so a
                  // wholly-failed import leaves the user on Home to read why.
                  const landed = result?.items.some(
                    (item) => item.status === "success",
                  );
                  if (landed) {
                    setActivePage("recordings");
                  }
                });
              }}
              isFetchingYoutube={youtubeQueue.activeCount > 0}
              youtubeItems={youtubeQueue.items}
              youtubeCurrentIndex={youtubeQueue.currentIndex}
              youtubeTotal={youtubeQueue.total}
              onEnqueueYoutube={youtubeQueue.enqueue}
              onRemoveYoutube={youtubeQueue.remove}
              youtubeFinishedCount={youtubeQueue.finishedCount}
              onClearFinishedYoutube={youtubeQueue.clearFinished}
              youtubeActiveProgress={youtubeQueue.activeProgress}
              onCancelYoutube={youtubeQueue.cancelActive}
              onView={openTranscriptViewer}
              onOpenLibrary={(filter) => {
                if (filter) {
                  setRecordingFilter(filter);
                }
                setActivePage("recordings");
              }}
            />
          ) : null}

          {activePage === "recordings" ? (
            <SavedRecordingsPage
              recordingActionMessage={recordingActionMessage}
              recentRecordings={bootstrap.recentRecordings}
              visibleRecordings={visibleRecordings}
              recordingFilter={recordingFilter}
              recordingFilterTabs={recordingFilterTabs}
              recordingPage={recordingPage}
              recordingPageCount={recordingPageCount}
              recordingSearch={recordingSearch}
              selectedRecordings={selectedRecordings}
              visibleSelectedPaths={visibleSelectedPaths}
              configuredAnkiDeckLabel={configuredAnkiDeckLabel}
              configuredDeckMenuOptions={configuredDeckMenuOptions}
              currentDeckName={settingsDraft.anki.deckName}
              currentNoteType={settingsDraft.anki.noteType}
              availableAnkiDecks={availableAnkiDecks}
              transcriptionLanguage={settingsDraft.whisper.language}
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
              onSearchChange={setRecordingSearch}
              onPageChange={setRecordingPage}
              onDefaultDeckChange={(deck) =>
                updateSettings({
                  anki: {
                    deckName: deck,
                  },
                })
              }
              onRefreshAnki={() =>
                void refreshAnkiCatalog(undefined, { notifySuccess: true })
              }
              onToggleSelection={toggleRecordingSelection}
              onClearSelection={clearRecordingSelection}
              onOpenRecordingMenuChange={setOpenRecordingMenuPath}
              onTranscribe={enqueueTranscriptions}
              onPushToAnki={pushRecordingsToAnki}
              onAddFurigana={addFuriganaToAnki}
              onTranslate={translateRecordings}
              onConvertToMp3={convertRecordingsToMp3}
              onDeleteRecording={deleteRecording}
              onDeleteRecordings={deleteRecordings}
              onView={openTranscriptViewer}
              transcriptionItems={transcriptionQueue.items}
              transcriptionActiveProgress={transcriptionQueue.activeProgress}
              transcriptionCurrentIndex={transcriptionQueue.currentIndex}
              transcriptionTotal={transcriptionQueue.total}
              transcriptionFinishedCount={transcriptionQueue.finishedCount}
              onCancelTranscription={transcriptionQueue.cancelActive}
              onRemoveTranscription={transcriptionQueue.remove}
              onClearFinishedTranscription={transcriptionQueue.clearFinished}
            />
          ) : null}

          {activePage === "transcript" ? (
            viewingRecording ? (
              <TranscriptViewerPage
                recording={viewingRecording}
                onBack={closeTranscriptViewer}
                onReTranscribe={(force) =>
                  enqueueTranscriptions([viewingRecording.filePath], force)
                }
                isReTranscribing={transcriptionQueue.items.some(
                  (item) =>
                    item.filePath === viewingRecording.filePath &&
                    (item.status === "queued" || item.status === "active"),
                )}
                onReTranslate={(force) =>
                  void translateRecordings([viewingRecording.filePath], force)
                }
                isReTranslating={busyAction === "translateRecording"}
                onMineSegment={async (text, startMs, endMs, translation) => {
                  const result = await mineSegment(
                    viewingRecording.filePath,
                    text,
                    startMs,
                    endMs,
                    translation,
                  );
                  const item = result?.items[0];
                  return Boolean(
                    item && item.status === "success" && item.noteId !== null,
                  );
                }}
                isMining={busyAction === "mineSegment"}
                expressionFieldMapped={expressionFieldMapped}
                ankiReachable={ankiReachable}
              />
            ) : (
              <div className="transcript-viewer">
                <div className="transcript-viewer-body is-single">
                  <div className="transcript-error">
                    <p className="panel-kicker">Recording unavailable</p>
                    <p>
                      This recording is no longer available. It may have been
                      deleted from this machine.
                    </p>
                    <button
                      type="button"
                      className="secondary"
                      onClick={closeTranscriptViewer}
                    >
                      Back to recordings
                    </button>
                  </div>
                </div>
              </div>
            )
          ) : null}

          {activePage === "setup" ? (
            <SetupChecklist
              steps={setupChecklist}
              summary={setupSummary}
              onOpenSection={openSettingsSection}
              onNavigate={setActivePage}
            />
          ) : null}

          <SettingsPages
            activePage={activePage}
            scrollTarget={settingsScrollTarget}
            onScrollTargetHandled={clearSettingsScrollTarget}
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
            ytdlpUpdateResult={ytdlpUpdateResult}
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
            onDownloadRecommendedYtdlp={downloadRecommendedYtdlp}
            onCheckYtdlpUpdate={checkYtdlpUpdate}
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
