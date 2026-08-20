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

  // Errors from the video library are notices about one action, not conditions of the app.
  // As cards they sat on the page until something else replaced them; as a toast the report
  // arrives, can be dismissed, and leaves. Longer than a success because a failure is worth
  // reading.
  function showError(message: string) {
    toast.error(message, { duration: 5000 });
    // Every failed command already funnels through here, so this is the one place that catches
    // them without fifty call sites each remembering to log.
    logToFile("ERROR", "action_failed", message);
  }

  // The engine reports a user Cancel as an ordinary Err carrying this exact string, so
  // without recognising it a deliberate stop arrived as a red failure toast reading
  // "transcription cancelled." — lowercase, an internal constant shown verbatim.
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

  // A recording's audio is RENAMED when its first transcript lands (the new stem is
  // derived from the transcript), so the path the viewer was opened with stops
  // resolving at exactly the moment a first-time transcription finishes. `createdAtMs`
  // survives the rename, so it is remembered while the lookup works and used to follow
  // the recording to its new path when it stops — otherwise watching a first
  // transcription live would end in "Recording unavailable".
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
    // `createdAtMs` is wall-clock milliseconds and is not enforced unique — a batch
    // import can stamp two files identically. Adopt only an unambiguous match: showing
    // the wrong recording's transcript would be worse than reporting it unavailable.
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
  const watch = useWatchSession();
  const watchSubtitles = useWatchSubtitles();
  // Rows mined in this watch session, and the per-mine padding overrides. "" means
  // "use the Settings value", resolved in Rust so a later settings change still applies.
  const [watchMinedKeys, setWatchMinedKeys] = useState<Set<string>>(() => new Set());
  const [watchMiningKey, setWatchMiningKey] = useState<string | null>(null);
  const [padBeforeMs, setPadBeforeMs] = useState("");
  const [padAfterMs, setPadAfterMs] = useState("");
  // The sidecar file the session was opened with. Remembered because alass rewrites a
  // subtitle FILE, and mpv's snapshot only reports the line on screen — an embedded track
  // has no path to hand it.
  const [watchSubtitlePath, setWatchSubtitlePath] = useState<string | null>(null);
  const [isSyncingSubtitles, setIsSyncingSubtitles] = useState(false);
  const [watchSyncResult, setWatchSyncResult] = useState<{
    ok: boolean;
    message: string;
  } | null>(null);

  // Realign the subtitle file against the video's own audio, then reload the corrected
  // file into both mpv (done in Rust, so the player never shows subtitles the app thinks
  // it has fixed) and the app's cue list.
  // Transcribe the picked video's own audio into a subtitle file, then adopt it as the
  // session's sidecar. Adopting it is the point: from there it is an ordinary subtitle file,
  // so the alass Sync button below applies to it exactly like a downloaded one — which is
  // what makes the transcribe-time-realign chain testable end to end.
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

  // The video library. Selection and the generate-progress live here rather than in WatchPage
  // so they survive leaving the page — the whole point of remembering a pairing is that it
  // outlives the visit that made it.
  const [generateProgress, setGenerateProgress] = useState<number | null>(null);
  const [missingVideoPaths, setMissingVideoPaths] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const watchedVideos = bootstrap.watchedVideos;

  // Check which remembered videos are still on disk, whenever the list changes. A missing file
  // dims its row rather than removing it: the row carries the subtitle mapping, and a
  // disconnected drive should not cost a pairing.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const missing = await invoke<string[]>("missing_watched_videos");
        if (!cancelled) {
          setMissingVideoPaths(new Set(missing));
        }
      } catch {
        // A failed check must not dim every row — better to show them all as present than
        // to tell the user their whole library has vanished because one call failed.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [watchedVideos]);

  // The generator reuses the library's transcription-progress channel, so the bar is fed by
  // the same event the queue uses. Only listened to while a generation is in flight, so a
  // library batch running elsewhere cannot paint this bar.
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
        // Playback has already started. Failing to note the timestamp is not worth an error
        // in front of someone who just pressed play.
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
    // Deliberately keyed on the message only: the same failure twice in a row is two
    // attempts and deserves to be reported twice.
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
        // Inside the try, not before it: the previous version awaited outside, so a rejected
        // confirm escaped as an unhandled promise and the whole action looked like a no-op.
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
      // alass's own report of what it shifted. Shown rather than swallowed: a sync can
      // succeed and still be wrong, and this is the only thing that says by how much.
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
    // A refusal — the whisper slot already taken, the engine not ready — used to reach the
    // user only as a "failed" chip with the reason in a tooltip.
    onFailure: showWarning,
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

  // A finished mic recording is now saved untranscribed and hands itself off for
  // transcription through this event, so auto-transcribe-after-recording runs on
  // A stale Anki mapping is said out loud, once per distinct problem. Everything else this
  // read can report — Anki closed, nothing mapped yet — stays quiet by design; those are
  // states the user is already in on purpose. This one looks identical from the outside
  // (no marks appear) and is the only one they can act on.
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

  // Read by the mined-line listener below, which must not re-subscribe every time the cue
  // list changes — an event arriving during that gap would be lost.
  const cuesRef = useRef(watchSubtitles.cues);
  cuesRef.current = watchSubtitles.cues;

  // A watch line was mined — mark its row, whichever of the three ways started it.
  //
  // The subtitle row used to record this itself, which is why only that one showed the
  // mark: the Mine button never recorded it, and the hotkey CANNOT, because it fires in
  // Rust while mpv has focus and never reaches this window. The backend emits at the one
  // point all three pass through, so the mark no longer depends on which control was used.
  useEffect(() => {
    const unlisten = listen<{ startMs: number; endMs: number; text: string }>(
      "watch-line-mined",
      ({ payload }) => {
        if (!payload) {
          return;
        }
        // Matched by TIME, not by rebuilding the row's key from the payload. mpv reports
        // the line it is currently showing, and neither its bounds nor its text have to
        // agree exactly with the cue our own parser produced — a millisecond of rounding,
        // or ASS line-break markup stripped differently, is enough to miss. The row to
        // mark is simply the one covering the moment that was mined.
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

  // the same non-blocking queue as a manual transcribe instead of blocking the app
  // with the full-screen overlay. `force = false`; the queue dedupes by file path,
  // so a duplicate event is a harmless no-op.
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

  // Sentence mining needs a mapped expression field to write to and a reachable
  // Anki. `offline` is the only catalog status that definitively means "not
  // reachable"; idle/ready are treated as reachable (the click still reports
  // honestly if Anki turns out to be down).
  const expressionFieldMapped = Boolean(settingsDraft.anki.fields.transcription);
  const ankiReachable = displayedAnkiCatalog.status !== "offline";

  // Reading the whole mining deck is too heavy to poll, so it is refreshed only when
  // the transcript viewer opens — the one place the marks are shown — and again after
  // a successful mine. `ankiReachable` is a dependency because starting Anki while a
  // transcript is already open must bring the marks in; without it the page would show
  // enabled Mine buttons and no marks at all until the user navigated away and back.
  // The viewed path is one for the same reason: today every recording switch goes
  // through the library, but a "next recording" control inside the viewer would
  // otherwise silently leave the marks stale.
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

  // App-wide, not per page: a word should be lookupable wherever it is read — a transcript,
  // the live transcript as it streams, or the watch subtitle list. The hook listens on the
  // document and finds its target by walking up from the pointer, so one instance covers
  // every surface and a second would fire every lookup twice.
  const lookup = useWordScanner({
    modifier: settingsDraft.scanner.modifier,
    releaseBehavior: settingsDraft.scanner.releaseBehavior,
    debounceMs: settingsDraft.scanner.debounceMs,
  });

  // Lines this session has turned into cards, however they were mined.
  //
  // App sees BOTH paths — a row's Mine button and the popup's — so one set covers
  // both, and word-mining a line the row already mined reads as already mined
  // rather than as a duplicate failure.
  //
  // Keyed by moment and text, the way the viewer keys its own markers, so editing a
  // line by merging or splitting it correctly reads as a different line.
  const [minedLineKeys, setMinedLineKeys] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const minedLineKey = (text: string, startMs: number, endMs: number) =>
    `${startMs}:${endMs}:${text}`;
  // The text back out of a key. Sliced past the second colon rather than split on it: the two
  // timestamps can never contain one, but a transcript line certainly can.
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
  //
  // Stable identity, because the page reports through an effect: a new function each
  // render would make that effect fire every render.
  const absorbMinedLines = useCallback((keys: ReadonlySet<string>) => {
    setMinedLineKeys((previous) => {
      const missing = [...keys].filter((key) => !previous.has(key));
      // Same set back when nothing is new, so React bails out of the re-render
      // rather than looping through the effect that called this.
      return missing.length === 0 ? previous : new Set([...previous, ...missing]);
    });
  }, []);

  // A different recording is a different set of lines; carrying the old keys over
  // would mark rows in the new one that were never mined.
  useEffect(() => {
    setMinedLineKeys(new Set());
  }, [viewingRecording?.filePath]);

  // Forget a line whose card is no longer in Anki.
  //
  // A key here is a memory of having MADE a card, not an observation that one exists. Delete
  // the card in Anki and the memory outlives it — and because this set survives navigation,
  // clearing it needed an app restart: a row stayed spent with nothing behind it.
  //
  // **No new scanning.** This runs on the read that already happens when the transcript opens,
  // and changes what that read means rather than how often anything reads. Deleting in Anki is
  // rare enough that stepping out of the transcript and back is a fair price for it.
  //
  // Gated on a read having SUCCEEDED. An empty `minedSentences` means "the deck is empty" and
  // "we could never look" alike — offline, unmapped and error all return nothing — so expiring
  // against one would wipe every marker the moment Anki closed, which is a far worse bug than
  // the one this fixes.
  useEffect(() => {
    if (minedReadCount === 0) {
      return;
    }
    setMinedLineKeys((previous) => {
      const survivors = [...previous].filter((key) =>
        minedSentences.has(normalizeSegmentText(sentenceOfMinedKey(key))),
      );
      // Same set back when nothing expired, so React bails out rather than re-rendering
      // every consumer on every read.
      return survivors.length === previous.size ? previous : new Set(survivors);
    });
  }, [minedReadCount, minedSentences]);

  // Mining a word from the popup is offered only when everything it needs is on
  // hand: a recording open with its audio still present, and a scanned line that
  // carries a moment. A translation row and a live transcript segment both have
  // text and nothing behind it, so neither gets the button rather than getting one
  // that fails.
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
    // The line goes in as the sentence, exactly as mining that row would send it,
    // so a word card and a sentence card from the same line are the same card plus
    // a word — and Anki's duplicate check sees the same first field either way.
    const result = await mineSegment(
      viewingRecording.filePath,
      text,
      startMs,
      endMs,
      null,
      word,
    );
    const item = result?.items[0];
    // Remembered on a duplicate as well as on success: the card exists either way,
    // and the button saying "Mine" next to a line that already has one is the thing
    // being fixed. The phrase is the one `user_friendly_anki_error` writes for
    // Anki's duplicate refusal — both ends of that string are ours.
    if (
      item &&
      (item.status === "success" || item.message.includes("already exists"))
    ) {
      rememberMinedLine(text, startMs, endMs);
    }
    // Left open on purpose, unlike before. The button turning to "Mined" IS the
    // confirmation, and closing the popup the instant it changes would hide it.
  };

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
                  // The shared group, so pressing this disables the six Settings download
                  // buttons and vice versa — which is what that group exists to do.
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
                // Records the open, and re-records the pairing this session actually used —
                // so the list's "opened" line is true and the mapping matches what played.
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
              // Available for any picked video, playing or not — a subtitle-free file is
              // usually discovered before pressing play.
              onGenerateSubtitles={(videoPath) =>
                void generateWatchSubtitles(videoPath)
              }
              // Only a sidecar file can be realigned: alass rewrites a subtitle file, and an
              // embedded track has none of its own.
              onSyncSubtitles={
                watchSubtitlePath && watch.snapshot.path
                  ? () => void syncWatchSubtitles()
                  : undefined
              }
              scanner={settingsDraft.scanner}
              onToggleOverlay={(enabled) => {
                updateSettings({ scanner: { overlayEnabled: enabled } });
                // The backend owns mpv's own subtitle visibility, so the toggle has to
                // reach it directly rather than waiting on the settings autosave.
                void invoke("set_scanner_overlay", { enabled });
              }}
              onStop={() => {
                setWatchSubtitlePath(null);
                setWatchSyncResult(null);
                // Take the overlay down with the video. A scanner window left tracking a
                // dead player is the one way it could end up stranded on screen.
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
              // The deck-wide marks reuse the same normalized set the transcript viewer
              // uses, so a line already in Anki is flagged here too.
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
                  // The mark and the deck refresh come from the `watch-line-mined` event,
                  // which every mining route emits — this one no longer records its own.
                  .finally(() =>
                    setWatchMiningKey((current) =>
                      current === key ? null : current,
                    ),
                  );
              }}
              onMerge={(index) =>
                watchSubtitles.merge(
                  index,
                  // CJK runs without inter-word spaces; a space would leave an
                  // unnatural gap in the merged sentence and on the card.
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
                // How the last run for THIS recording ended. Without it, cancelling from
                // inside the viewer just drops the live pane and lands on an empty
                // transcript — indistinguishable from whisper having crashed, or from a
                // recording that was never transcribed at all.
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
                    // Keep the persistent set in step with the card that was just
                    // written, so the mark survives leaving and reopening the viewer.
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
                // Cancel is offered only while THIS recording is the active run —
                // cancelActive kills whatever whisper is working on, so exposing it
                // for a queued-but-not-started file would stop the wrong one.
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
            onRefreshKnownWords={refreshKnownWords}
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
          // `useAppViewState` stamps the resolved theme on <html>; reading it here beats
          // threading the same value down to one attribute.
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
