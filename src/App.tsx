import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import * as TooltipPrimitive from "@radix-ui/react-tooltip";
import { Toaster, toast } from "sonner";
import { HomePage } from "./components/home/HomePage";
import { HomeSetupCard } from "./components/home/HomeSetupCard";
import { PageSidebar } from "./components/layout/PageSidebar";
import { SavedRecordingsPage } from "./components/recordings/SavedRecordingsPage";
import { SettingsPages } from "./components/settings/SettingsPages";
import { SetupChecklist } from "./components/settings/SetupChecklist";
import { TranscriptViewerPage } from "./components/transcripts/TranscriptViewerPage";
import { WatchPage } from "./components/watch/WatchPage";
import { JimakuDialog } from "./components/watch/JimakuDialog";
import { useConfirm } from "./components/ui/ConfirmDialogProvider";
import { LookupPopup } from "./components/scanner/LookupPopup";
import { useWordScanner } from "./hooks/useWordScanner";
import { BusyOverlay } from "./components/ui/BusyOverlay";
import { useAnkiCatalog } from "./hooks/useAnkiCatalog";
import { useAppBootstrap } from "./hooks/useAppBootstrap";
import { useAppViewState } from "./hooks/useAppViewState";
import { useMinedSentences } from "./hooks/useMinedSentences";
import { useProgress } from "./hooks/useProgress";
import { ProgressPage } from "./components/progress/ProgressPage";
import type { Range } from "./components/progress/DayChart";
import type { Lens } from "./components/progress/lenses";
import { useWatchSession } from "./hooks/useWatchSession";
import { useWatchSubtitles } from "./hooks/useWatchSubtitles";
import { segmentMineKey } from "./lib/segments";
import { normalizeSegmentText } from "./components/transcripts/transcriptText";
import { useRecordingActions } from "./hooks/useRecordingActions";
import { useRecordingLibrary } from "./hooks/useRecordingLibrary";
import { useRecorderActions } from "./hooks/useRecorderActions";
import { useSetupActions } from "./hooks/useSetupActions";
import { useTranscriptionQueue } from "./hooks/useTranscriptionQueue";
import { useYoutubeQueue } from "./hooks/useYoutubeQueue";
import { fileNameFromPath } from "./lib/format";
import { logToFile } from "./lib/log";
import { isDownloadBusy } from "./types";
import type {
  AppBootstrap,
  AppPage,
  BusyAction,
  SettingsSection,
  SubtitleOrigin,
  WhisperAssetUpdateResult,
} from "./types";

function App() {
  const confirmDialog = useConfirm();
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

  function showError(message: string) {
    toast.error(message, { duration: 5000 });
    logToFile("ERROR", "action_failed", message);
  }

  // Settings shows the outcome beside its button; the Progress page has only this to say it.
  async function refreshWordListFromProgress() {
    const snapshot = await refreshKnownWords();
    if (snapshot === null) {
      return;
    }
    if (snapshot.status === "ready") {
      showSuccess(`Your word list now has ${snapshot.wordCount.toLocaleString()} words.`);
    } else if (snapshot.status === "offline") {
      showWarning("Anki isn't open, so your word list wasn't refreshed.");
    } else {
      showWarning(snapshot.message);
    }
  }

  const TRANSCRIPTION_CANCELLED = "transcription cancelled.";

  function reportCancellable(caught: unknown, fallback: string, cancelledMessage: string) {
    const message =
      caught instanceof Error ? caught.message : String(caught ?? fallback);
    if (message.trim().toLowerCase() === TRANSCRIPTION_CANCELLED) {
      toast(cancelledMessage, { duration: 3500 });
      return;
    }
    showError(message);
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

  const viewedCreatedAtRef = useRef<number | null>(null);
  const viewingRecording = (() => {
    if (viewingRecordingPath === null) {
      viewedCreatedAtRef.current = null;
      return null;
    }
    const byPath = bootstrap.recentRecordings.find(
      (recording) => recording.filePath === viewingRecordingPath,
    );
    if (byPath) {
      viewedCreatedAtRef.current = byPath.createdAtMs;
      return byPath;
    }
    const createdAtMs = viewedCreatedAtRef.current;
    if (createdAtMs === null) {
      return null;
    }

    const matches = bootstrap.recentRecordings.filter(
      (recording) => recording.createdAtMs === createdAtMs,
    );
    return matches.length === 1 ? matches[0] : null;
  })();

  // Adopt the new path once the rename is observed, so every later lookup (and the
  // live-segment / cancel matching, which compare paths) goes back to a direct hit.
  useEffect(() => {
    if (
      viewingRecording &&
      viewingRecording.filePath !== viewingRecordingPath &&
      viewingRecordingPath !== null
    ) {
      setViewingRecordingPath(viewingRecording.filePath);
    }
  }, [viewingRecording, viewingRecordingPath]);

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
  const { minedSentences, minedWarning, minedReadCount, refreshMinedSentences } =
    useMinedSentences();
  const {
    report: progressReport,
    readCount: progressReadCount,
    failed: progressFailed,
    minedCards: progressMinedCards,
    countingCards: progressCountingCards,
    countCards: countProgressCards,
  } = useProgress(activePage);
  // Held here rather than on the page, so leaving Progress does not reset what it shows.
  const [progressLens, setProgressLens] = useState<Lens>("time");
  const [progressRange, setProgressRange] = useState<Range>("month");
  const watch = useWatchSession();
  const watchSubtitles = useWatchSubtitles();
  const [watchMinedKeys, setWatchMinedKeys] = useState<Set<string>>(() => new Set());
  const [watchMiningKey, setWatchMiningKey] = useState<string | null>(null);
  const [padBeforeMs, setPadBeforeMs] = useState("");
  const [padAfterMs, setPadAfterMs] = useState("");
  const [watchSubtitlePath, setWatchSubtitlePath] = useState<string | null>(null);
  const [isSyncingSubtitles, setIsSyncingSubtitles] = useState(false);
  const [watchSyncResult, setWatchSyncResult] = useState<{
    ok: boolean;
    message: string;
  } | null>(null);

  // Realign the subtitle file against the video's own audio, then reload the corrected
  // file into both mpv (done in Rust, so the player never shows subtitles the app thinks
  // it has fixed) and the app's cue list.
  const [isGeneratingSubtitles, setIsGeneratingSubtitles] = useState(false);
  const generateWatchSubtitles = useCallback(
    async (videoPath: string) => {
      setIsGeneratingSubtitles(true);
      setGeneratingPath(videoPath);
      setWatchSyncResult(null);
      try {
        const generated = await invoke<{
          path: string;
          cueCount: number;
          language: string;
        }>("generate_watch_subtitles", { videoPath });
        setWatchSubtitlePath(generated.path);
        if (watch.snapshot.path === videoPath) {
          await watchSubtitles.load(videoPath, generated.path, null);
        }
        showSuccess(
          `${generated.cueCount} lines written to ${fileNameFromPath(
            generated.path,
          )}. Realign it if the timings look off.`,
        );
      } catch (caught) {
        reportCancellable(
          caught,
          "Subtitles could not be generated.",
          "Subtitle generation cancelled.",
        );
      } finally {
        setIsGeneratingSubtitles(false);
        setGeneratingPath(null);
      }
    },
    [watch.snapshot.path, watchSubtitles],
  );

  const [generateProgress, setGenerateProgress] = useState<number | null>(null);
  const [missingVideoPaths, setMissingVideoPaths] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const watchedVideos = bootstrap.watchedVideos;

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const missing = await invoke<string[]>("missing_watched_videos");
        if (!cancelled) {
          setMissingVideoPaths(new Set(missing));
        }
      } catch {
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [watchedVideos]);

  useEffect(() => {
    if (!isGeneratingSubtitles) {
      setGenerateProgress(null);
      return;
    }
    const unlisten = listen<number>("transcription-progress", (event) => {
      setGenerateProgress(event.payload);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [isGeneratingSubtitles]);

  const setWatchedVideoOpened = useCallback(
    async (videoPath: string) => {
      try {
        applyBootstrap(
          await invoke<AppBootstrap>("mark_watched_video_opened", { videoPath }),
        );
      } catch {
      }
    },
    [applyBootstrap],
  );

  const [videoSearch, setVideoSearch] = useState("");
  const [openVideoMenuPath, setOpenVideoMenuPath] = useState<string | null>(null);
  const [jimakuDialogPath, setJimakuDialogPath] = useState<string | null>(null);
  const [generatingPath, setGeneratingPath] = useState<string | null>(null);

  // Filtering on the title the row actually shows, so what you type matches what you read.
  const visibleVideos = useMemo(() => {
    const query = videoSearch.trim().toLowerCase();
    if (!query) {
      return watchedVideos;
    }
    return watchedVideos.filter((video) =>
      (video.title ?? video.videoPath).toLowerCase().includes(query),
    );
  }, [watchedVideos, videoSearch]);

  const realignWatchedVideo = useCallback(
    async (videoPath: string) => {
      const video = watchedVideos.find((entry) => entry.videoPath === videoPath);
      if (!video?.subtitlePath) {
        return;
      }
      setIsSyncingSubtitles(true);
      setWatchSyncResult(null);
      const pending = toast.loading("Realigning subtitles…");
      try {
        const synced = await invoke<{ path: string; summary: string }>(
          "sync_watch_subtitles",
          { videoPath, subtitlePath: video.subtitlePath },
        );
        // The backend has already repointed the mapping; this refreshes the list from it.
        applyBootstrap(await invoke<AppBootstrap>("get_app_bootstrap"));
        showSuccess(
          `Realigned as ${fileNameFromPath(synced.path)}.${
            synced.summary ? ` ${synced.summary}` : ""
          }`,
        );
      } catch (caught) {
        showError(
          caught instanceof Error
            ? caught.message
            : String(caught ?? "The subtitles could not be realigned."),
        );
      } finally {
        toast.dismiss(pending);
        setIsSyncingSubtitles(false);
      }
    },
    [applyBootstrap, watchedVideos],
  );

  useEffect(() => {
    if (watch.error) {
      showError(watch.error);
    }
  }, [watch.error]);

  const addWatchedVideo = useCallback(
    async (videoPath: string) => {
      const pending = toast.loading("Adding video…");
      try {
        applyBootstrap(
          await invoke<AppBootstrap>("add_watched_video", { videoPath }),
        );
      } catch (caught) {
        showError(
          caught instanceof Error
            ? caught.message
            : String(caught ?? "The video could not be added."),
        );
      } finally {
        toast.dismiss(pending);
      }
    },
    [applyBootstrap],
  );

  const setWatchedVideoSubtitle = useCallback(
    async (videoPath: string, subtitlePath: string | null, origin: string | null) => {
      try {
        applyBootstrap(
          await invoke<AppBootstrap>("set_watched_video_subtitle", {
            videoPath,
            subtitlePath,
            origin,
          }),
        );
      } catch (caught) {
        showError(
          caught instanceof Error
            ? caught.message
            : String(caught ?? "The subtitle could not be saved."),
        );
      }
    },
    [applyBootstrap],
  );

  const forgetWatchedVideo = useCallback(
    async (videoPath: string) => {
      try {
        const confirmed = await confirmDialog({
          title: "Remove this video?",
          message:
            "Remove this video from the list, along with the subtitles it is paired with. The video file itself is not deleted.",
          okLabel: "Remove",
          cancelLabel: "Keep",
          danger: true,
        });
        if (!confirmed) {
          return;
        }
        applyBootstrap(
          await invoke<AppBootstrap>("forget_watched_video", { videoPath }),
        );
      } catch (caught) {
        showError(
          caught instanceof Error
            ? caught.message
            : String(caught ?? "The video could not be removed."),
        );
      }
    },
    [applyBootstrap, confirmDialog],
  );

  const syncWatchSubtitles = useCallback(async () => {
    const videoPath = watch.snapshot.path;
    if (!watchSubtitlePath || !videoPath) {
      return;
    }
    setIsSyncingSubtitles(true);
    setWatchSyncResult(null);
    try {
      const synced = await invoke<{ path: string; summary: string }>(
        "sync_watch_subtitles",
        { videoPath, subtitlePath: watchSubtitlePath },
      );
      setWatchSubtitlePath(synced.path);
      await watchSubtitles.load(videoPath, synced.path, null);
      setWatchSyncResult({
        ok: true,
        message: `Saved as ${fileNameFromPath(synced.path)} and loaded.${
          synced.summary ? ` ${synced.summary}` : ""
        }`,
      });
    } catch (caught) {
      setWatchSyncResult({
        ok: false,
        message:
          caught instanceof Error ? caught.message : String(caught ?? "Sync failed."),
      });
    } finally {
      setIsSyncingSubtitles(false);
    }
  }, [watch.snapshot.path, watchSubtitlePath, watchSubtitles]);

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
    isDownloadingAssets,
    isRecording,
    manualRuntimeOverride,
    modelDiskSize,
    modelInstalled,
    modelLabel,
    recorderBusy,
    resolvedCliPath,
    resolvedModelPath,
    runtimeInstalled,
    setupChecklist,
    setupEntry,
    setupIncomplete,
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
    reinstallFfmpeg,
    downloadRecommendedModel,
    downloadWhisperVadModel,
    downloadRecommendedRuntime,
    downloadRecommendedYtdlp,
    downloadRecommendedAlass,
    downloadRecommendedMpv,
    reinstallMpv,
    downloadRecommendedDictionary,
    downloadMissingEssentials,
    refreshKnownWords,
    scanVocabularySources,
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
    showError,
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

  const youtubeQueue = useYoutubeQueue({
    importYoutube,
    onAllComplete: (landed) => {
      if (landed > 0) {
        setActivePage("recordings");
      }
    },
  });

  const transcriptionQueue = useTranscriptionQueue({
    applyBootstrap,
    persistSettingsIfNeeded,
    onFailure: showWarning,
  });

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

  const reportedMinedWarningRef = useRef<string | null>(null);
  useEffect(() => {
    if (minedWarning && reportedMinedWarningRef.current !== minedWarning) {
      reportedMinedWarningRef.current = minedWarning;
      showWarning(minedWarning);
    }
    if (!minedWarning) {
      reportedMinedWarningRef.current = null;
    }
  }, [minedWarning]);


  const cuesRef = useRef(watchSubtitles.cues);
  cuesRef.current = watchSubtitles.cues;

  // A watch line was mined — mark its row, whichever of the three ways started it.
  useEffect(() => {
    const unlisten = listen<{ startMs: number; endMs: number; text: string }>(
      "watch-line-mined",
      ({ payload }) => {
        if (!payload) {
          return;
        }
        const midpoint =
          payload.startMs + (payload.endMs - payload.startMs) / 2;
        const cue = cuesRef.current.find(
          (candidate) =>
            midpoint >= candidate.startMs && midpoint < candidate.endMs,
        );
        setWatchMinedKeys((previous) =>
          new Set(previous).add(
            cue
              ? segmentMineKey(cue)
              : segmentMineKey({
                  startMs: payload.startMs,
                  endMs: payload.endMs,
                  text: payload.text,
                }),
          ),
        );
        void refreshMinedSentences();
      },
    );
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [refreshMinedSentences]);

  useEffect(() => {
    const unlisten = listen<{ filePath: string; title?: string }>(
      "recording-transcribe-request",
      ({ payload }) => {
        if (!payload?.filePath) {
          return;
        }
        transcriptionQueue.enqueue(
          [{ filePath: payload.filePath, title: payload.title }],
          false,
        );
      },
    );
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [transcriptionQueue.enqueue]);

  const expressionFieldMapped = Boolean(settingsDraft.anki.fields.transcription);
  const ankiReachable = displayedAnkiCatalog.status !== "offline";

  useEffect(() => {
    if (activePage === "transcript") {
      void refreshMinedSentences();
    }
  }, [
    activePage,
    ankiReachable,
    viewingRecording?.filePath,
    refreshMinedSentences,
  ]);

  const lookup = useWordScanner({
    modifier: settingsDraft.scanner.modifier,
    releaseBehavior: settingsDraft.scanner.releaseBehavior,
    debounceMs: settingsDraft.scanner.debounceMs,
  });

  // Lines this session has turned into cards, however they were mined.
  const [minedLineKeys, setMinedLineKeys] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const minedLineKey = (text: string, startMs: number, endMs: number) =>
    `${startMs}:${endMs}:${text}`;
  const sentenceOfMinedKey = (key: string) => {
    const firstColon = key.indexOf(":");
    const secondColon = key.indexOf(":", firstColon + 1);
    return secondColon === -1 ? key : key.slice(secondColon + 1);
  };
  const rememberMinedLine = (text: string, startMs: number, endMs: number) => {
    setMinedLineKeys((previous) => new Set(previous).add(minedLineKey(text, startMs, endMs)));
  };

  // Everything the transcript page has mined, folded in so the popup knows about it
  // too — a line mined by "Mine all" is as much a card as one mined from the popup.
  const absorbMinedLines = useCallback((keys: ReadonlySet<string>) => {
    setMinedLineKeys((previous) => {
      const missing = [...keys].filter((key) => !previous.has(key));
      return missing.length === 0 ? previous : new Set([...previous, ...missing]);
    });
  }, []);

  useEffect(() => {
    setMinedLineKeys(new Set());
  }, [viewingRecording?.filePath]);

  // Forget a line whose card is no longer in Anki.
  useEffect(() => {
    if (minedReadCount === 0) {
      return;
    }
    setMinedLineKeys((previous) => {
      const survivors = [...previous].filter((key) =>
        minedSentences.has(normalizeSegmentText(sentenceOfMinedKey(key))),
      );
      return survivors.length === previous.size ? previous : new Set(survivors);
    });
  }, [minedReadCount, minedSentences]);

  const scannedLine = lookup.target;
  const canMineScannedWord = Boolean(
    viewingRecording &&
      !viewingRecording.audioDeleted &&
      scannedLine &&
      scannedLine.startMs !== undefined &&
      scannedLine.endMs !== undefined,
  );

  const mineScannedWord = async (word: string) => {
    if (!viewingRecording || !scannedLine) {
      return;
    }
    const { startMs, endMs, text } = scannedLine;
    if (startMs === undefined || endMs === undefined || !text) {
      return;
    }
    const result = await mineSegment(
      viewingRecording.filePath,
      text,
      startMs,
      endMs,
      null,
      word,
    );
    const item = result?.items[0];
    if (
      item &&
      (item.status === "success" || item.message.includes("already exists"))
    ) {
      rememberMinedLine(text, startMs, endMs);
    }
  };

  return (
    <main className="app-shell">
      <TooltipPrimitive.Provider delayDuration={180}>
        <Toaster
          position="top-right"
          richColors
          closeButton
          expand
          toastOptions={{
            className: "app-toast",
          }}
        />

      {bootstrap.loggingFailure ? (
        <section className="banner banner-error">
          Logging is unavailable, so a problem report from this session will be
          incomplete. {bootstrap.loggingFailure}
        </section>
      ) : null}
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
            workflowPages={workflowPages}
            setupEntry={setupEntry}
            onPageSelect={setActivePage}
          />

          <section className="content-column">
          {activePage === "home" ? (
            <HomePage
              setupCard={
                <HomeSetupCard
                  setupIncomplete={setupIncomplete}
                  requirements={bootstrap.transcriptionRequirements}
                  modelReady={modelInstalled}
                  modelLabel={modelLabel}
                  modelDiskSize={modelDiskSize}
                  isDownloadingAssets={isDownloadingAssets}
                  downloadIsActive={downloadIsActive}
                  downloadSnapshot={bootstrap.modelDownload}
                  downloadBusy={isDownloadBusy(busyAction)}
                  onDownloadMissing={() => void downloadMissingEssentials()}
                  onTogglePause={() => void toggleDownloadPause()}
                  onCancelDownload={() => void cancelDownload()}
                />
              }
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
              onReTranscribe={(files) => enqueueTranscriptions(files, true)}
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

          {jimakuDialogPath ? (
        <JimakuDialog
          videoPath={jimakuDialogPath}
          hasApiKey={settingsDraft.jimakuApiKey.trim().length > 0}
          onDownloaded={(subtitlePath) =>
            void setWatchedVideoSubtitle(jimakuDialogPath, subtitlePath, "jimaku")
          }
          onClose={() => setJimakuDialogPath(null)}
          onOpenSettings={() => openSettingsSection("scanner")}
        />
      ) : null}

      {activePage === "watch" ? (
            <WatchPage
              snapshot={watch.snapshot}
              startingPath={watch.startingPath}
              onStart={(videoPath, subtitlePath) => {
                setWatchMinedKeys(new Set());
                setWatchSubtitlePath(subtitlePath);
                setWatchSyncResult(null);
                void watch.start(videoPath, subtitlePath);
                void watchSubtitles.load(videoPath, subtitlePath, null);
                void setWatchedVideoOpened(videoPath);
              }}
              onSetSubtitleDelay={(delayMs) => void watch.setSubtitleDelay(delayMs)}
              hasJimakuKey={settingsDraft.jimakuApiKey.trim().length > 0}
              isSyncing={isSyncingSubtitles}
              syncResult={watchSyncResult}
              videos={visibleVideos}
              onAddVideo={(videoPath) => void addWatchedVideo(videoPath)}
              onSearchJimaku={setJimakuDialogPath}
              onRealign={(videoPath) => void realignWatchedVideo(videoPath)}
              generatingPath={generatingPath}
              openMenuPath={openVideoMenuPath}
              onOpenMenuChange={setOpenVideoMenuPath}
              searchQuery={videoSearch}
              onSearchChange={setVideoSearch}
              onSubtitleChosen={(videoPath, subtitlePath, origin: SubtitleOrigin) =>
                void setWatchedVideoSubtitle(videoPath, subtitlePath, origin)
              }
              onForgetVideo={(videoPath) => void forgetWatchedVideo(videoPath)}
              missingVideoPaths={missingVideoPaths}
              generateProgress={generateProgress}
              onCancelGenerate={() => {
                void emit("transcription-cancel");
              }}
              onGenerateSubtitles={(videoPath) =>
                void generateWatchSubtitles(videoPath)
              }
              onSyncSubtitles={
                watchSubtitlePath && watch.snapshot.path
                  ? () => void syncWatchSubtitles()
                  : undefined
              }
              scanner={settingsDraft.scanner}
              onToggleOverlay={(enabled) => {
                updateSettings({ scanner: { overlayEnabled: enabled } });
                void invoke("set_scanner_overlay", { enabled });
              }}
              onStop={() => {
                setWatchSubtitlePath(null);
                setWatchSyncResult(null);
                void invoke("set_scanner_overlay", { enabled: false });
                void watch.stop();
                watchSubtitles.clear();
                setWatchMinedKeys(new Set());
              }}
              onMine={() => void watch.mine()}
              isMining={watch.isMining}
              mineResult={watch.mineResult}
              mineHotkey={bootstrap.shell.hotkeys.mine || null}
              cues={watchSubtitles.cues}
              subtitlesError={watchSubtitles.error}
              minedKeys={watchMinedKeys}
              deckMinedKeys={
                new Set(
                  watchSubtitles.cues
                    .filter((cue) =>
                      minedSentences.has(normalizeSegmentText(cue.text)),
                    )
                    .map(segmentMineKey),
                )
              }
              miningKey={watchMiningKey}
              mineDisabledReason={
                !expressionFieldMapped
                  ? "Map an Anki note first"
                  : !ankiReachable
                    ? "Anki not reachable"
                    : null
              }
              onSeek={(positionMs) => void watch.seek(positionMs)}
              onMineLine={(index) => {
                const cue = watchSubtitles.cues[index];
                const videoPath = watch.snapshot.path;
                if (!cue || !videoPath) {
                  return;
                }
                const key = segmentMineKey(cue);
                setWatchMiningKey(key);
                void watch
                  .mineLine(
                    videoPath,
                    cue.text,
                    cue.startMs,
                    cue.endMs,
                    padBeforeMs === "" ? null : Number(padBeforeMs),
                    padAfterMs === "" ? null : Number(padAfterMs),
                  )
                  .finally(() =>
                    setWatchMiningKey((current) =>
                      current === key ? null : current,
                    ),
                  );
              }}
              onMerge={(index) =>
                watchSubtitles.merge(
                  index,
                  /[぀-ヿ㐀-鿿]/.test(
                    watchSubtitles.cues[index]?.text ?? "",
                  )
                    ? ""
                    : " ",
                )
              }
              onSplit={(index) => watchSubtitles.split(index)}
              padBeforeMs={padBeforeMs}
              padAfterMs={padAfterMs}
              onPadBeforeChange={setPadBeforeMs}
              onPadAfterChange={setPadAfterMs}
            />
          ) : null}

          {activePage === "progress" ? (
            <ProgressPage
              bootstrap={bootstrap}
              report={progressReport}
              readCount={progressReadCount}
              failed={progressFailed}
              minedCards={progressMinedCards}
              countingCards={progressCountingCards}
              onCountCards={() => void countProgressCards()}
              refreshingWordList={busyAction === "refreshKnownWords"}
              onRefreshWordList={() => void refreshWordListFromProgress()}
              onGoToStudyPicks={() => openSettingsSection("studyPicks")}
              lens={progressLens}
              onLens={setProgressLens}
              range={progressRange}
              onRange={setProgressRange}
            />
          ) : null}

          {activePage === "transcript" ? (
            viewingRecording ? (
              <TranscriptViewerPage
                recording={viewingRecording}
                transcriptionLanguage={settingsDraft.whisper.language}
                clipPaddingMs={settingsDraft.anki.clipPaddingMs}
                mineWordsWithoutContext={
                  settingsDraft.features.mineWordsWithoutContext
                }
                allowDuplicateMinedWords={
                  settingsDraft.features.allowDuplicateMinedWords
                }
                externallyMinedKeys={minedLineKeys}
                onLinesMined={absorbMinedLines}
          knownWordsBuiltAtMs={bootstrap.knownWords.builtAtMs}
                onBack={closeTranscriptViewer}
                onReTranscribe={(force) =>
                  enqueueTranscriptions([viewingRecording.filePath], force)
                }
                isReTranscribing={transcriptionQueue.items.some(
                  (item) =>
                    item.filePath === viewingRecording.filePath &&
                    (item.status === "queued" || item.status === "active"),
                )}
                reTranscribeProgress={
                  transcriptionQueue.items.some(
                    (item) =>
                      item.filePath === viewingRecording.filePath &&
                      item.status === "active",
                  )
                    ? transcriptionQueue.activeProgress
                    : null
                }
                lastTranscriptionOutcome={(() => {
                  const item = [...transcriptionQueue.items]
                    .reverse()
                    .find(
                      (candidate) =>
                        candidate.filePath === viewingRecording.filePath,
                    );
                  return item &&
                    (item.status === "cancelled" || item.status === "failed")
                    ? { status: item.status, message: item.message }
                    : null;
                })()}
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
                  const mined = Boolean(
                    item && item.status === "success" && item.noteId !== null,
                  );
                  if (mined) {
                    void refreshMinedSentences();
                  }
                  return mined;
                }}
                isMining={busyAction === "mineSegment"}
                expressionFieldMapped={expressionFieldMapped}
                ankiReachable={ankiReachable}
                minedSentences={minedSentences}
                liveSegments={
                  transcriptionQueue.activeSegments.filePath ===
                  viewingRecording.filePath
                    ? transcriptionQueue.activeSegments.segments
                    : []
                }
                onCancelTranscription={
                  transcriptionQueue.items.some(
                    (item) =>
                      item.filePath === viewingRecording.filePath &&
                      item.status === "active",
                  )
                    ? transcriptionQueue.cancelActive
                    : undefined
                }
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
            onShowError={showError}
            onBrowseFile={browseForFile}
            onCheckRuntimeUpdate={checkRuntimeUpdate}
            onDownloadRuntimeVersion={downloadRuntimeVersion}
            onDownloadRecommendedRuntime={downloadRecommendedRuntime}
            onCheckModelUpdate={checkModelUpdate}
            onDownloadRecommendedModel={downloadRecommendedModel}
            onDownloadWhisperVadModel={downloadWhisperVadModel}
            onDownloadRecommendedFfmpeg={downloadRecommendedFfmpeg}
            onReinstallFfmpeg={reinstallFfmpeg}
            onDownloadRecommendedYtdlp={downloadRecommendedYtdlp}
            onDownloadRecommendedAlass={downloadRecommendedAlass}
            onDownloadRecommendedMpv={downloadRecommendedMpv}
            onReinstallMpv={reinstallMpv}
            onDownloadRecommendedDictionary={downloadRecommendedDictionary}
            onRefreshKnownWords={async () => {
              await refreshKnownWords();
            }}
            onScanVocabularySources={scanVocabularySources}
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

      {lookup.target ? (
        <LookupPopup
          anchor={lookup.target.anchor}
          result={lookup.result}
          isLoading={lookup.isLoading}
          error={lookup.error}
          theme={document.documentElement.dataset.theme === "light" ? "light" : "dark"}
          fontFamily={settingsDraft.scanner.fontFamily}
          fontSizePx={settingsDraft.scanner.fontSizePx}
          onClose={lookup.close}
          onMine={canMineScannedWord ? mineScannedWord : undefined}
          isMining={busyAction === "mineSegment"}
          isMined={
            scannedLine?.startMs !== undefined && scannedLine.endMs !== undefined
              ? minedLineKeys.has(
                  minedLineKey(
                    scannedLine.text,
                    scannedLine.startMs,
                    scannedLine.endMs,
                  ),
                )
              : false
          }
        />
      ) : null}
    </main>
  );
}

export default App;
