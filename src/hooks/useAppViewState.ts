import { useEffect, useState } from "react";
import {
  MODEL_OPTIONS,
  RECOMMENDED_RUNTIME_VERSION,
} from "../constants";
import { whisperStatusLabel } from "../lib/helpers";
import {
  activePageLabel,
  createDetailPages,
  createSetupPages,
  createWorkflowPages,
} from "../lib/navigation";
import type {
  AppBootstrap,
  AppPage,
  AppSettings,
  BusyAction,
  AnkiCatalog,
} from "../types";

type UseAppViewStateOptions = {
  activePage: AppPage;
  ankiCatalog: AnkiCatalog;
  bootstrap: AppBootstrap;
  busyAction: BusyAction;
  settingsDraft: AppSettings;
};

export function useAppViewState({
  activePage,
  ankiCatalog,
  bootstrap,
  busyAction,
  settingsDraft,
}: UseAppViewStateOptions) {
  const [systemTheme, setSystemTheme] = useState<"light" | "dark">(() =>
    window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light",
  );
  const [clockMs, setClockMs] = useState(() => Date.now());
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
  const workflowPages = createWorkflowPages(bootstrap.recentRecordings.length);
  const setupPages = createSetupPages({
    whisperStatus: whisperStatusLabel(bootstrap.whisperDetection.status),
    runtimeVersion: activeRuntimeVersion,
    modelLabel: selectedModel.label,
    ffmpegReady: bootstrap.ffmpegDetection.status === "ready",
    ankiReady: ankiCatalog.status === "ready",
  });
  const detailPages = createDetailPages();
  const currentPageLabel = activePageLabel(activePage, [
    ...workflowPages,
    ...setupPages,
    ...detailPages,
  ]);

  return {
    activeRuntimeVersion,
    busyOverlayLabel,
    currentPageLabel,
    downloadIsActive,
    elapsedRecordingMs,
    hotkeyTooltip,
    installedRuntimeVersions,
    isRecording,
    manualRuntimeOverride,
    modelInstalled,
    resolvedCliPath,
    resolvedModelPath,
    recorderBusy,
    runtimeInstalled,
    setupPages,
    showBusyOverlay,
    workflowPages,
  };
}
