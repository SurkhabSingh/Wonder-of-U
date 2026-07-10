import { useCallback, useEffect, useMemo, useState } from "react";
import {
  pathHasExtension,
  recordingAnkiPushForTarget,
  recordingHasTranscriptForLanguage,
  recordingSupportsFurigana,
} from "../lib/helpers";
import { createRecordingFilterTabs } from "../lib/navigation";
import type {
  AnkiCatalog,
  AnkiSettings,
  RecentRecording,
  RecordingFilter,
} from "../types";

type UseRecordingLibraryOptions = {
  ankiCatalog: AnkiCatalog;
  ankiSettings: AnkiSettings;
  recentRecordings: RecentRecording[];
  transcriptionLanguage: string;
};

const RECORDINGS_PER_PAGE = 8;

function mergeSavedAnkiSettingsIntoCatalog(
  catalog: AnkiCatalog,
  ankiSettings: AnkiSettings,
): AnkiCatalog {
  const savedFields = Object.values(ankiSettings.fields).filter(Boolean);

  return {
    ...catalog,
    decks: Array.from(
      new Set([
        ...(ankiSettings.deckName ? [ankiSettings.deckName] : []),
        ...catalog.decks,
      ]),
    ),
    noteTypes: Array.from(
      new Set([
        ...(ankiSettings.noteType ? [ankiSettings.noteType] : []),
        ...catalog.noteTypes,
      ]),
    ),
    fields: Array.from(new Set([...savedFields, ...catalog.fields])),
    message:
      catalog.status === "idle" && (ankiSettings.deckName || ankiSettings.noteType)
        ? "Using your saved Anki mapping. Refresh only if you changed decks, note types, or fields in Anki."
        : catalog.message,
  };
}

export function useRecordingLibrary({
  ankiCatalog,
  ankiSettings,
  recentRecordings,
  transcriptionLanguage,
}: UseRecordingLibraryOptions) {
  const [selectedRecordings, setSelectedRecordings] = useState<string[]>([]);
  const [recordingFilter, setRecordingFilter] = useState<RecordingFilter>("all");
  const [recordingPage, setRecordingPage] = useState(1);
  const [openRecordingMenuPath, setOpenRecordingMenuPath] = useState<string | null>(
    null,
  );

  const displayedAnkiCatalog = useMemo(
    () => mergeSavedAnkiSettingsIntoCatalog(ankiCatalog, ankiSettings),
    [ankiCatalog, ankiSettings],
  );

  const transcribedRecordings = useMemo(
    () =>
      recentRecordings.filter((recording) =>
        recordingHasTranscriptForLanguage(recording, transcriptionLanguage),
      ),
    [recentRecordings, transcriptionLanguage],
  );

  const untranscribedRecordings = useMemo(
    () =>
      recentRecordings.filter(
        (recording) =>
          !recordingHasTranscriptForLanguage(recording, transcriptionLanguage),
      ),
    [recentRecordings, transcriptionLanguage],
  );

  const recordingPushedToDeck = useCallback(
    (recording: RecentRecording, deckName: string) =>
      recordingAnkiPushForTarget(
        recording,
        transcriptionLanguage,
        deckName,
        ankiSettings.noteType,
      ) !== null,
    [ankiSettings.noteType, transcriptionLanguage],
  );

  const recordingPushableToDeck = useCallback(
    (recording: RecentRecording, deckName: string) =>
      deckName.trim().length > 0 &&
      recordingHasTranscriptForLanguage(recording, transcriptionLanguage) &&
      !recording.audioDeleted &&
      !recordingPushedToDeck(recording, deckName),
    [recordingPushedToDeck, transcriptionLanguage],
  );

  const recordingPushedToCurrentAnkiDeck = useCallback(
    (recording: RecentRecording) =>
      recordingPushedToDeck(recording, ankiSettings.deckName),
    [ankiSettings.deckName, recordingPushedToDeck],
  );

  const pushableRecordings = useMemo(
    () =>
      transcribedRecordings.filter(
        (recording) =>
          !recording.audioDeleted && !recordingPushedToCurrentAnkiDeck(recording),
      ),
    [recordingPushedToCurrentAnkiDeck, transcribedRecordings],
  );

  const untranslatedRecordings = useMemo(
    () =>
      transcribedRecordings.filter(
        (recording) => recording.translationPath === null,
      ),
    [transcribedRecordings],
  );

  const completeRecordings = useMemo(
    () =>
      recentRecordings.filter(
        (recording) =>
          Boolean(recording.transcriptPath) &&
          recordingPushedToCurrentAnkiDeck(recording) &&
          recording.translationPath !== null,
      ),
    [recentRecordings, recordingPushedToCurrentAnkiDeck],
  );

  const filteredRecordings = useMemo(() => {
    if (recordingFilter === "needsTranscription") {
      return untranscribedRecordings;
    }

    if (recordingFilter === "needsAnki") {
      return pushableRecordings;
    }

    if (recordingFilter === "needsTranslation") {
      return untranslatedRecordings;
    }

    if (recordingFilter === "complete") {
      return completeRecordings;
    }

    return recentRecordings;
  }, [
    completeRecordings,
    pushableRecordings,
    recentRecordings,
    recordingFilter,
    untranslatedRecordings,
    untranscribedRecordings,
  ]);

  const recordingPageCount = Math.max(
    1,
    Math.ceil(filteredRecordings.length / RECORDINGS_PER_PAGE),
  );
  const recordingPageStart =
    filteredRecordings.length === 0
      ? 0
      : (recordingPage - 1) * RECORDINGS_PER_PAGE + 1;
  const recordingPageEnd = Math.min(
    recordingPage * RECORDINGS_PER_PAGE,
    filteredRecordings.length,
  );
  const visibleRecordings = useMemo(() => {
    const startIndex = (recordingPage - 1) * RECORDINGS_PER_PAGE;
    return filteredRecordings.slice(
      startIndex,
      startIndex + RECORDINGS_PER_PAGE,
    );
  }, [filteredRecordings, recordingPage]);

  const selectedRecordingSet = useMemo(
    () => new Set(selectedRecordings),
    [selectedRecordings],
  );

  const visibleSelectedRecordings = useMemo(
    () =>
      filteredRecordings.filter((recording) =>
        selectedRecordingSet.has(recording.filePath),
      ),
    [filteredRecordings, selectedRecordingSet],
  );

  const visibleSelectedPaths = useMemo(
    () => visibleSelectedRecordings.map((recording) => recording.filePath),
    [visibleSelectedRecordings],
  );

  const useBatchActionsOnly = visibleSelectedPaths.length > 1;

  const selectedTranscribedRecordings = useMemo(
    () =>
      visibleSelectedRecordings.filter((recording) =>
        recordingHasTranscriptForLanguage(recording, transcriptionLanguage),
      ),
    [transcriptionLanguage, visibleSelectedRecordings],
  );

  const selectedPushableRecordings = useMemo(
    () =>
      selectedTranscribedRecordings.filter((recording) =>
        recordingPushableToDeck(recording, ankiSettings.deckName),
      ),
    [ankiSettings.deckName, recordingPushableToDeck, selectedTranscribedRecordings],
  );

  const selectedUntranscribedRecordings = useMemo(
    () =>
      visibleSelectedRecordings.filter(
        (recording) =>
          !recordingHasTranscriptForLanguage(recording, transcriptionLanguage),
      ),
    [transcriptionLanguage, visibleSelectedRecordings],
  );

  const selectedUntranslatedRecordings = useMemo(
    () =>
      selectedTranscribedRecordings.filter(
        (recording) => recording.translationPath === null,
      ),
    [selectedTranscribedRecordings],
  );

  const selectedFuriganaRecordings = useMemo(
    () =>
      selectedTranscribedRecordings.filter(
        (recording) => {
          const push = recordingAnkiPushForTarget(
            recording,
            transcriptionLanguage,
            ankiSettings.deckName,
            ankiSettings.noteType,
          );
          return (
            push !== null &&
            !push.furiganaApplied &&
            recordingSupportsFurigana(recording, transcriptionLanguage)
          );
        },
      ),
    [
      ankiSettings.deckName,
      ankiSettings.noteType,
      selectedTranscribedRecordings,
      transcriptionLanguage,
    ],
  );

  const convertibleRecordings = useMemo(
    () =>
      recentRecordings.filter(
        (recording) =>
          !recording.audioDeleted &&
          recording.transcriptPath !== null &&
          pathHasExtension(recording.filePath, "wav"),
      ),
    [recentRecordings],
  );

  const selectedConvertibleRecordings = useMemo(
    () =>
      visibleSelectedRecordings.filter(
        (recording) =>
          !recording.audioDeleted &&
          recording.transcriptPath !== null &&
          pathHasExtension(recording.filePath, "wav"),
      ),
    [visibleSelectedRecordings],
  );

  const recordingFilterTabs = useMemo(
    () =>
      createRecordingFilterTabs({
        allCount: recentRecordings.length,
        untranscribedCount: untranscribedRecordings.length,
        pushableCount: pushableRecordings.length,
        untranslatedCount: untranslatedRecordings.length,
        completeCount: completeRecordings.length,
      }),
    [
      completeRecordings.length,
      pushableRecordings.length,
      recentRecordings.length,
      untranslatedRecordings.length,
      untranscribedRecordings.length,
    ],
  );

  const configuredAnkiDeckLabel =
    ankiSettings.deckName.trim() || "No deck selected";

  const availableAnkiDecks = useMemo(
    () => displayedAnkiCatalog.decks.filter((deck) => deck.trim().length > 0),
    [displayedAnkiCatalog.decks],
  );

  const configuredDeckMenuOptions =
    availableAnkiDecks.length > 0
      ? availableAnkiDecks
      : ankiSettings.deckName
        ? [ankiSettings.deckName]
        : [];

  const selectedRecordingsPushableToDeck = useCallback(
    (deckName: string) =>
      selectedTranscribedRecordings.filter((recording) =>
        recordingPushableToDeck(recording, deckName),
      ),
    [recordingPushableToDeck, selectedTranscribedRecordings],
  );

  const toggleRecordingSelection = useCallback((filePath: string) => {
    setSelectedRecordings((current) =>
      current.includes(filePath)
        ? current.filter((selectedPath) => selectedPath !== filePath)
        : [...current, filePath],
    );
  }, []);

  const clearRecordingSelection = useCallback(() => {
    setSelectedRecordings([]);
  }, []);

  const changeRecordingFilter = useCallback((filter: RecordingFilter) => {
    setRecordingFilter(filter);
    setRecordingPage(1);
    setOpenRecordingMenuPath(null);
  }, []);

  useEffect(() => {
    if (useBatchActionsOnly) {
      setOpenRecordingMenuPath(null);
    }
  }, [useBatchActionsOnly]);

  useEffect(() => {
    setRecordingPage((current) => Math.min(current, recordingPageCount));
  }, [recordingPageCount]);

  useEffect(() => {
    setOpenRecordingMenuPath(null);
  }, [recordingPage]);

  useEffect(() => {
    setSelectedRecordings((current) =>
      current.filter((filePath) =>
        recentRecordings.some((recording) => recording.filePath === filePath),
      ),
    );
  }, [recentRecordings]);

  return {
    availableAnkiDecks,
    completeRecordings,
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
    recordingPageEnd,
    recordingPageStart,
    filteredRecordingsCount: filteredRecordings.length,
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
    setRecordingFilter: changeRecordingFilter,
    setRecordingPage,
    toggleRecordingSelection,
    transcribedRecordings,
    untranslatedRecordings,
    untranscribedRecordings,
    useBatchActionsOnly,
    visibleRecordings,
    visibleSelectedPaths,
  };
}
