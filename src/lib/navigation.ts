import type {
  AppPage,
  RecordingFilter,
  SettingsSection,
  TranscriptionRequirement,
} from "../types";

export type PageNavigationItem = {
  id: AppPage;
  label: string;
  description: string;
  count?: string;
};

export type RecordingFilterTab = {
  id: RecordingFilter;
  label: string;
  count: number;
};

// The two libraries are named for what they hold. "Library" alone promised everything and
// held only audio, which is exactly why a video felt like it belonged there.
export function createWorkflowPages(
  recordingCount: number,
  videoCount: number,
): PageNavigationItem[] {
  return [
    { id: "home", label: "Home", description: "Your work at a glance" },
    {
      id: "recordings",
      label: "Audio library",
      description: "",
      count: String(recordingCount),
    },
    {
      id: "watch",
      label: "Video library",
      description: "Play a video and mine as you go",
      count: String(videoCount),
    },
    { id: "progress", label: "Progress", description: "How much you can read" },
  ];
}

export type SetupChecklistStep = {
  id: string;
  target: SettingsSection;
  label: string;
  description: string;
  done: boolean | null;
  required: boolean;
  value?: string | null;
};

export type SetupChecklistSummary = {
  total: number;
  done: number;
  allDone: boolean;
};

// The Setup checklist ("setup") and the single Settings page ("settings") both
// live behind the sidebar's "Setup" entry.
export const SETUP_PAGE_IDS: AppPage[] = ["setup", "settings"];

export function isSetupPage(page: AppPage): boolean {
  return SETUP_PAGE_IDS.includes(page);
}

/**
 * Which Setup rows are required, taken from what transcription actually demands.
 */
function requirementLookup(
  transcriptionRequirements: TranscriptionRequirement[],
): (id: string) => boolean {
  const required = new Set(transcriptionRequirements.map((entry) => entry.id));
  return (id) => required.has(id);
}

export function createSetupChecklist({
  cliReady,
  modelReady,
  ffmpegReady,
  ytdlpReady,
  ankiConfigured,
  transcriptionRequirements,
  runtimeVersion,
  modelLabel,
  ankiSummary,
  themeLabel,
}: {
  cliReady: boolean;
  modelReady: boolean;
  ffmpegReady: boolean;
  ytdlpReady: boolean;
  ankiConfigured: boolean;
  transcriptionRequirements: TranscriptionRequirement[];
  runtimeVersion?: string | null;
  modelLabel?: string | null;
  ankiSummary?: string | null;
  themeLabel?: string | null;
}): SetupChecklistStep[] {
  const isRequired = requirementLookup(transcriptionRequirements);
  return [
    {
      id: "runtime",
      target: "whisper",
      label: "Whisper CLI",
      description: cliReady
        ? "Runtime installed"
        : "Install the Whisper runtime",
      done: cliReady,
      required: isRequired("whisper"),
      value: cliReady ? runtimeVersion ?? null : null,
    },
    {
      id: "model",
      target: "whisper",
      label: "Whisper Model",
      description: modelReady
        ? "Model downloaded"
        : "Download a transcription model",
      done: modelReady,
      required: isRequired("whisper"),
      value: modelReady ? modelLabel ?? null : null,
    },
    {
      id: "anki",
      target: "anki",
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
      target: "whisper",
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
      target: "storage",
      label: "Audio Processing",
      description: ffmpegReady
        ? "FFmpeg ready"
        : "Install FFmpeg — transcription and MP3 conversion both need it",
      done: ffmpegReady,
      required: isRequired("ffmpeg"),
    },
    {
      id: "ytdlp",
      target: "storage",
      label: "Link Import",
      description: ytdlpReady
        ? "yt-dlp ready"
        : "Install yt-dlp to import from a link",
      done: ytdlpReady,
      required: false,
    },
    {
      id: "preferences",
      target: "preferences",
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
    description: summary.allDone ? "Setup complete" : "",
    count: summary.allDone ? undefined : `${summary.done}/${summary.total}`,
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
