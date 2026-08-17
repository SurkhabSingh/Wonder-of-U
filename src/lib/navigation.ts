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
  // Optional count shown as a `.status-chip-count` badge beside the label
  // (Library item total, Setup steps-done fraction).
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
  ];
}

export type SetupChecklistStep = {
  id: string;
  // The Settings section this row deep-links to. Multiple rows (CLI, model,
  // status) point at the single "whisper" section.
  target: SettingsSection;
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

// The Setup checklist ("setup") and the single Settings page ("settings") both
// live behind the sidebar's "Setup" entry. This group keeps that entry
// highlighted while the user drills into a Settings section from the checklist.
export const SETUP_PAGE_IDS: AppPage[] = ["setup", "settings"];

export function isSetupPage(page: AppPage): boolean {
  return SETUP_PAGE_IDS.includes(page);
}

/**
 * Which Setup rows are required, taken from what transcription actually demands.
 *
 * The backend answers this — `transcription_requirements` — and the answer arrives on the
 * bootstrap. It is read rather than restated because restating it is how FFmpeg came to be
 * described here as "Install FFmpeg for optional MP3 conversion" while transcription would not
 * run without it: a new user could finish every required step and still do nothing.
 *
 * A row whose id is not a requirement is genuinely optional. An unknown id defaults to optional
 * rather than required, so a requirement the backend adds and this file has no row for shows up
 * as a missing row rather than a step nobody can complete.
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
      // Both this row and the model below map onto the one "whisper" requirement: detection
      // reports the CLI and the model as a pair, and the checklist splits them only so the
      // user can see which half is missing.
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
      // Was "MP3 Compression", described as optional. FFmpeg is what decodes audio for
      // Whisper, so nothing transcribes without it; MP3 conversion is the smaller of the two
      // things it does and was the only one this row mentioned.
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
      label: "YouTube Import",
      description: ytdlpReady
        ? "yt-dlp ready"
        : "Install yt-dlp for optional YouTube import",
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
