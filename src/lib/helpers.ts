import { LANGUAGE_OPTIONS } from "../constants";
import type { RecentRecording } from "../types";

export function pathHasExtension(path: string, extension: string): boolean {
  return path.toLowerCase().endsWith(`.${extension.toLowerCase()}`);
}

export function recordingSupportsFurigana(recording: RecentRecording): boolean {
  const language = recording.transcriptLanguage?.toLowerCase();
  return language === "ja" || language === "japanese";
}

export function transcriptLanguageLabel(language: string | null): string | null {
  if (!language) {
    return null;
  }

  return (
    LANGUAGE_OPTIONS.find((option) => option.code === language)?.label ??
    language.toUpperCase()
  );
}

export function normalizeSelection(
  selection: string | string[] | null,
): string | null {
  if (!selection) {
    return null;
  }

  return Array.isArray(selection) ? selection[0] ?? null : selection;
}

export function whisperStatusLabel(status: string): string {
  switch (status) {
    case "ready":
      return "Ready";
    case "cliMissing":
      return "CLI Missing";
    case "modelMissing":
      return "Model Missing";
    case "invalid":
      return "Invalid";
    default:
      return "Needs Setup";
  }
}
