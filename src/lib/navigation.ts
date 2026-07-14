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
    { id: "home", label: "Home", description: "Your work at a glance" },
    {
      id: "recordings",
      label: "Library",
      description: `${recordingCount} local item${recordingCount === 1 ? "" : "s"}`,
    },
  ];
}

export type SetupChecklistStep = {
  id: AppPage;
  label: string;
  description: string;
  done: boolean | null;
  required: boolean;
  // The step's current configured value (model name, deck, runtime version,
  // theme), shown in place of the generic description so the checklist reads
  // as a status board. Null falls back to the description.
  value?: string | null;
};

export type SetupChecklistSummary = {
  total: number;
  done: number;
  allDone: boolean;
};

// Every setup surface lives behind the single "Setup" checklist entry. This
// group drives the sidebar's active-state highlight when the user drills into
// one of the individual settings pages from the checklist.
export const SETUP_PAGE_IDS: AppPage[] = [
  "setup",
  "preferences",
  "whisper",
  "runtime",
  "model",
  "storage",
  "anki",
];

export function isSetupPage(page: AppPage): boolean {
  return SETUP_PAGE_IDS.includes(page);
}

export function createSetupChecklist({
  cliReady,
  modelReady,
  ffmpegReady,
  ankiConfigured,
  runtimeVersion,
  modelLabel,
  ankiSummary,
  themeLabel,
}: {
  cliReady: boolean;
  modelReady: boolean;
  ffmpegReady: boolean;
  ankiConfigured: boolean;
  runtimeVersion?: string | null;
  modelLabel?: string | null;
  ankiSummary?: string | null;
  themeLabel?: string | null;
}): SetupChecklistStep[] {
  return [
    {
      id: "runtime",
      label: "Whisper CLI",
      description: cliReady
        ? "Runtime installed"
        : "Install the Whisper runtime",
      done: cliReady,
      required: true,
      value: cliReady ? runtimeVersion ?? null : null,
    },
    {
      id: "model",
      label: "Whisper Model",
      description: modelReady
        ? "Model downloaded"
        : "Download a transcription model",
      done: modelReady,
      required: true,
      value: modelReady ? modelLabel ?? null : null,
    },
    {
      id: "anki",
      label: "Anki Mapping",
      description: ankiConfigured
        ? "Note fields mapped"
        : "Map your Anki note fields",
      done: ankiConfigured,
      required: true,
      value: ankiConfigured ? ankiSummary ?? null : null,
    },
    {
      id: "whisper",
      label: "Whisper Status",
      description:
        cliReady && modelReady
          ? "Ready to transcribe"
          : "Waiting on the CLI and model",
      done: cliReady && modelReady,
      required: false,
    },
    {
      id: "storage",
      label: "MP3 Compression",
      description: ffmpegReady
        ? "FFmpeg ready"
        : "Install FFmpeg for optional MP3 conversion",
      done: ffmpegReady,
      required: false,
    },
    {
      id: "preferences",
      label: "App Preferences",
      description: "Theme, folders, and feature toggles",
      done: null,
      required: false,
      value: themeLabel ?? null,
    },
  ];
}

export function summarizeSetupChecklist(
  steps: SetupChecklistStep[],
): SetupChecklistSummary {
  const required = steps.filter((step) => step.required);
  const done = required.filter((step) => step.done === true).length;
  return {
    total: required.length,
    done,
    allDone: required.length > 0 && done === required.length,
  };
}

export function createSetupEntry(
  summary: SetupChecklistSummary,
): PageNavigationItem {
  return {
    id: "setup",
    label: "Setup",
    description: summary.allDone
      ? "Setup complete"
      : `${summary.done} of ${summary.total} steps done`,
  };
}

export function createDetailPages(): PageNavigationItem[] {
  return [
    {
      id: "transcript",
      label: "Transcript",
      description: "Read transcript and translation",
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
  return pages.find((page) => page.id === activePage)?.label ?? "Home";
}
