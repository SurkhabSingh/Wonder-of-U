import { LANGUAGE_OPTIONS } from "../constants";
import type { RecentRecording, RecordingAnkiPush } from "../types";

export function pathHasExtension(path: string, extension: string): boolean {
  return path.toLowerCase().endsWith(`.${extension.toLowerCase()}`);
}

export function normalizeTranscriptLanguage(language: string): string {
  return language.trim().toLowerCase() || "auto";
}

export function recordingHasTranscriptForLanguage(
  recording: RecentRecording,
  language: string,
): boolean {
  const normalizedLanguage = normalizeTranscriptLanguage(language);
  return recording.transcripts.some(
    (transcript) =>
      normalizeTranscriptLanguage(transcript.language) === normalizedLanguage,
  );
}

export function recordingAnkiPushForTarget(
  recording: RecentRecording,
  language: string,
  deckName: string,
  noteType: string,
): RecordingAnkiPush | null {
  const normalizedLanguage = normalizeTranscriptLanguage(language);
  const push = recording.ankiPushes.find(
    (candidate) =>
      normalizeTranscriptLanguage(candidate.language) === normalizedLanguage &&
      candidate.deckName === deckName &&
      candidate.noteType === noteType,
  );
  if (push) {
    return push;
  }

  if (
    recording.ankiPushes.length === 0 &&
    recording.ankiNoteId !== null &&
    recording.ankiDeckName === deckName &&
    recording.ankiNoteType === noteType
  ) {
    return {
      language: normalizedLanguage,
      deckName,
      noteType,
      noteId: recording.ankiNoteId,
      furiganaApplied: recording.furiganaApplied,
    };
  }

  return null;
}

export function recordingSupportsFurigana(
  recording: RecentRecording,
  language?: string,
): boolean {
  const transcripts = language
    ? recording.transcripts.filter(
        (transcript) =>
          normalizeTranscriptLanguage(transcript.language) ===
          normalizeTranscriptLanguage(language),
      )
    : recording.transcripts;

  if (
    transcripts.some((transcript) => {
      const requestedLanguage = normalizeTranscriptLanguage(transcript.language);
      const detectedLanguage = transcript.detectedLanguage?.toLowerCase();
      return (
        requestedLanguage === "ja" ||
        requestedLanguage === "japanese" ||
        detectedLanguage === "ja" ||
        detectedLanguage === "japanese"
      );
    })
  ) {
    return true;
  }

  if (recording.transcripts.length > 0) {
    return false;
  }

  const legacyLanguage = recording.transcriptLanguage?.toLowerCase();
  return legacyLanguage === "ja" || legacyLanguage === "japanese";
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

export function recordingTranscriptLanguageLabels(
  recording: RecentRecording,
): string[] {
  const labels = recording.transcripts.map((transcript) => {
    const requestedLanguage = normalizeTranscriptLanguage(transcript.language);
    const requestedLabel =
      transcriptLanguageLabel(requestedLanguage) ?? requestedLanguage.toUpperCase();
    const detectedLabel = transcriptLanguageLabel(transcript.detectedLanguage);

    return requestedLanguage === "auto" && detectedLabel
      ? `${requestedLabel} (${detectedLabel})`
      : requestedLabel;
  });

  if (labels.length === 0) {
    const legacyLabel = transcriptLanguageLabel(recording.transcriptLanguage);
    return legacyLabel ? [legacyLabel] : [];
  }

  return Array.from(new Set(labels));
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
