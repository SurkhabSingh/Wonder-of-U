import { useMemo, useState } from "react";
import { useRecordingTexts } from "../../hooks/useRecordingTexts";
import {
  formatBytes,
  formatDuration,
  formatTimestamp,
} from "../../lib/format";
import { transcriptLanguageLabel } from "../../lib/helpers";
import type { RecentRecording, RecordingTextDocument } from "../../types";
import { TranscriptLanguageTabs } from "./TranscriptLanguageTabs";
import type { TranscriptLanguageTab } from "./TranscriptLanguageTabs";
import { TranscriptReadingPane } from "./TranscriptReadingPane";
import { countMatches, splitTranscriptSegments } from "./transcriptText";

type TranscriptViewMode = "sideBySide" | "transcript" | "translation";

const VIEW_MODES: { id: TranscriptViewMode; label: string }[] = [
  { id: "sideBySide", label: "Side by side" },
  { id: "transcript", label: "Transcript" },
  { id: "translation", label: "Translation" },
];

// Scripts without word spacing get a wider leading and a shorter measure.
const CJK_LANGUAGES = new Set(["ja", "zh", "yue", "zh-cn", "zh-tw"]);

function documentLanguageLabel(document: RecordingTextDocument): string {
  const requested =
    transcriptLanguageLabel(document.language) ??
    document.language.toUpperCase();
  if (document.language === "auto") {
    return transcriptLanguageLabel(document.detectedLanguage) ?? requested;
  }
  return requested;
}

function isCjkDocument(document: RecordingTextDocument | null): boolean {
  if (!document) {
    return false;
  }
  return (
    CJK_LANGUAGES.has(document.language) ||
    (document.detectedLanguage !== null &&
      CJK_LANGUAGES.has(document.detectedLanguage))
  );
}

function documentMatchCount(
  document: RecordingTextDocument | null,
  query: string,
): number {
  if (!document || document.missing) {
    return 0;
  }
  return splitTranscriptSegments(document.text).reduce(
    (total, segment) => total + countMatches(segment, query),
    0,
  );
}

function TranscriptSkeleton() {
  return (
    <div className="transcript-pane">
      <div className="transcript-pane-body">
        <div className="transcript-skeleton" aria-hidden="true">
          {[72, 96, 58, 88, 66, 92, 50].map((width, index) => (
            <span
              key={index}
              className="transcript-skeleton-line"
              style={{ width: `${width}%` }}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

export function TranscriptViewerPage({
  recording,
  onBack,
}: {
  recording: RecentRecording;
  onBack: () => void;
}) {
  const changeSignature = `${recording.transcripts.length}:${
    recording.translationPath ?? ""
  }`;
  const { data, status, error, reload } = useRecordingTexts({
    filePath: recording.filePath,
    changeSignature,
  });

  const transcripts = data?.transcripts ?? [];
  const translations = data?.translations ?? [];

  const [activeLanguage, setActiveLanguage] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<TranscriptViewMode>("sideBySide");
  const [query, setQuery] = useState("");
  const [selectedSegment, setSelectedSegment] = useState<string | null>(null);

  const activeTranscript = useMemo<RecordingTextDocument | null>(() => {
    if (transcripts.length === 0) {
      return null;
    }
    if (activeLanguage) {
      const match = transcripts.find((doc) => doc.language === activeLanguage);
      if (match) {
        return match;
      }
    }
    return transcripts[0];
  }, [transcripts, activeLanguage]);

  const activeTranslation = translations[0] ?? null;

  const languageTabs = useMemo<TranscriptLanguageTab[]>(
    () =>
      transcripts.map((doc) => ({
        code: doc.language,
        label: documentLanguageLabel(doc),
      })),
    [transcripts],
  );

  const matchCount = useMemo(() => {
    const documents: (RecordingTextDocument | null)[] =
      viewMode === "transcript"
        ? [activeTranscript]
        : viewMode === "translation"
          ? [activeTranslation]
          : [activeTranscript, activeTranslation];
    return documents.reduce(
      (total, doc) => total + documentMatchCount(doc, query),
      0,
    );
  }, [viewMode, activeTranscript, activeTranslation, query]);

  const metaText = [
    formatDuration(recording.durationMs),
    formatBytes(recording.bytesWritten),
    formatTimestamp(recording.createdAtMs),
  ].join(" · ");

  const transcriptNote =
    activeTranscript &&
    activeTranscript.language === "auto" &&
    activeTranscript.detectedLanguage
      ? "Auto-detected"
      : null;

  const trimmedQuery = query.trim();

  return (
    <div className="transcript-viewer">
      <header className="transcript-viewer-header">
        <div className="transcript-viewer-heading">
          <button
            type="button"
            className="ghost transcript-back"
            onClick={onBack}
          >
            {"←"} Back to recordings
          </button>
          <div className="transcript-viewer-title">
            <p className="panel-kicker">Transcript</p>
            <h2 title={recording.fileName}>{recording.fileName}</h2>
            <p className="transcript-viewer-meta">{metaText}</p>
          </div>
        </div>

        <div className="transcript-viewer-controls">
          {languageTabs.length >= 2 ? (
            <TranscriptLanguageTabs
              value={activeTranscript?.language ?? ""}
              tabs={languageTabs}
              onChange={setActiveLanguage}
            />
          ) : null}

          <div
            className="transcript-view-toggle"
            role="group"
            aria-label="Reading layout"
          >
            {VIEW_MODES.map((mode) => (
              <button
                key={mode.id}
                type="button"
                className={`transcript-view-toggle-button ${
                  viewMode === mode.id ? "is-active" : ""
                }`}
                aria-pressed={viewMode === mode.id}
                onClick={() => setViewMode(mode.id)}
              >
                {mode.label}
              </button>
            ))}
          </div>

          <div className="transcript-find">
            <input
              type="search"
              className="transcript-find-input"
              placeholder="Find in transcript"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              aria-label="Find in transcript"
            />
            {trimmedQuery ? (
              <span className="transcript-find-count">
                {matchCount} match{matchCount === 1 ? "" : "es"}
              </span>
            ) : null}
          </div>
        </div>
      </header>

      {status === "error" ? (
        <div className="transcript-viewer-body is-single">
          <div className="transcript-error">
            <p className="panel-kicker">Could not load</p>
            <p>{error}</p>
            <button type="button" className="secondary" onClick={reload}>
              Try again
            </button>
          </div>
        </div>
      ) : status === "loading" || data === null ? (
        <div className="transcript-viewer-body is-single">
          <TranscriptSkeleton />
        </div>
      ) : (
        <div
          className={`transcript-viewer-body ${
            viewMode === "sideBySide" ? "is-split" : "is-single"
          }`}
        >
          {viewMode !== "translation" ? (
            <TranscriptReadingPane
              paneKey="transcript"
              kicker="Transcript"
              title={
                activeTranscript
                  ? documentLanguageLabel(activeTranscript)
                  : "Transcript"
              }
              note={transcriptNote}
              isCjk={isCjkDocument(activeTranscript)}
              document={activeTranscript}
              query={query}
              emptyLabel="No transcript text yet."
              missingLabel="The transcript file is missing from this machine."
              selectedSegment={selectedSegment}
              onSelectSegment={setSelectedSegment}
            />
          ) : null}

          {viewMode !== "transcript" ? (
            <TranscriptReadingPane
              paneKey="translation"
              kicker="Translation"
              title={
                activeTranslation
                  ? documentLanguageLabel(activeTranslation)
                  : "Not translated"
              }
              note={null}
              isCjk={isCjkDocument(activeTranslation)}
              document={activeTranslation}
              query={query}
              emptyLabel="No translation yet. Use Translate on the recording to create one."
              missingLabel="The translation file is missing from this machine."
              selectedSegment={selectedSegment}
              onSelectSegment={setSelectedSegment}
            />
          ) : null}
        </div>
      )}
    </div>
  );
}
