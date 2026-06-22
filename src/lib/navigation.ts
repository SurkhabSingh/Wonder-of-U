import type { AppPage, RecordingFilter } from "../types";

export type PageNavigationItem = {
  id: AppPage;
  label: string;
  description: string;
};

export type RecordingFilterTab = {
  id: RecordingFilter;
  label: string;
  count: number;
};

export function createWorkflowPages(recordingCount: number): PageNavigationItem[] {
  return [
    { id: "recorder", label: "Recorder", description: "Capture system audio" },
    {
      id: "recordings",
      label: "Saved Recordings",
      description: `${recordingCount} local item${recordingCount === 1 ? "" : "s"}`,
    },
  ];
}

export function createSetupPages({
  whisperStatus,
  runtimeVersion,
  modelLabel,
  ffmpegReady,
  ankiReady,
}: {
  whisperStatus: string;
  runtimeVersion: string;
  modelLabel: string;
  ffmpegReady: boolean;
  ankiReady: boolean;
}): PageNavigationItem[] {
  return [
    { id: "preferences", label: "App Preferences", description: "Theme and folders" },
    {
      id: "whisper",
      label: "Whisper Status",
      description: whisperStatus,
    },
    { id: "runtime", label: "Whisper CLI", description: runtimeVersion },
    { id: "model", label: "Whisper Model", description: modelLabel },
    {
      id: "storage",
      label: "MP3 Compression",
      description: ffmpegReady ? "FFmpeg ready" : "FFmpeg missing",
    },
    {
      id: "anki",
      label: "Anki Mapping",
      description: ankiReady ? "Connected" : "Saved mapping",
    },
  ];
}

export function createRecordingFilterTabs({
  allCount,
  untranscribedCount,
  pushableCount,
  untranslatedCount,
  completeCount,
}: {
  allCount: number;
  untranscribedCount: number;
  pushableCount: number;
  untranslatedCount: number;
  completeCount: number;
}): RecordingFilterTab[] {
  return [
    { id: "all", label: "All", count: allCount },
    {
      id: "needsTranscription",
      label: "Needs transcript",
      count: untranscribedCount,
    },
    { id: "needsAnki", label: "Needs Anki", count: pushableCount },
    {
      id: "needsTranslation",
      label: "Needs translation",
      count: untranslatedCount,
    },
    { id: "complete", label: "Complete", count: completeCount },
  ];
}

export function activePageLabel(
  activePage: AppPage,
  pages: PageNavigationItem[],
): string {
  return pages.find((page) => page.id === activePage)?.label ?? "Recorder";
}
