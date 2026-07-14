import { useEffect, useMemo, useState } from "react";
import { useAudioPlayer } from "../../hooks/useAudioPlayer";
import { useRecordingTexts } from "../../hooks/useRecordingTexts";
import {
  formatBytes,
  formatDuration,
  formatTimestamp,
} from "../../lib/format";
import { transcriptLanguageLabel } from "../../lib/helpers";
import type {
  RecentRecording,
  RecordingSegment,
  RecordingTextDocument,
} from "../../types";
import { NowPlayingBar } from "../audio/NowPlayingBar";
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

// Sentence-ending punctuation used to pick a natural split point (CJK + Latin).
const SENTENCE_ENDINGS = new Set([
  "。",
  "！",
  "？",
  "．",
  ".",
  "!",
  "?",
  "…",
]);

// A stable, content-derived key for a segment so an already-mined row keeps its
// "✓ Mined" marker across re-renders. Merging/splitting produces a new sentence
// (new text/timing), so its key differs and the marker naturally resets.
function segmentMineKey(segment: RecordingSegment): string {
  return `${segment.startMs}:${segment.endMs}:${segment.text}`;
}

// Merge row i with row i+1 into one sentence spanning both time ranges. The
// joiner is script-aware: CJK scripts run without inter-word spaces, so a space
// would leave an unnatural gap in the merged sentence (and in a mined card).
function mergeSegmentAt(
  segments: RecordingSegment[],
  index: number,
  joiner: string,
): RecordingSegment[] {
  if (index < 0 || index >= segments.length - 1) {
    return segments;
  }
  const a = segments[index];
  const b = segments[index + 1];
  const merged: RecordingSegment = {
    text: `${a.text}${joiner}${b.text}`,
    startMs: a.startMs,
    endMs: b.endMs,
  };
  return [...segments.slice(0, index), merged, ...segments.slice(index + 2)];
}

// Split row i at the first sentence-ending punctuation at or after the text
// midpoint, else at the character midpoint. Time is divided proportionally by
// the character cut index so each half keeps a plausible span.
function splitSegmentAt(
  segments: RecordingSegment[],
  index: number,
): RecordingSegment[] {
  const segment = segments[index];
  if (!segment) {
    return segments;
  }
  const text = segment.text;
  if (text.length < 2) {
    return segments;
  }

  const midpoint = Math.floor(text.length / 2);
  let cutIndex = midpoint;
  for (let position = midpoint; position < text.length; position += 1) {
    if (SENTENCE_ENDINGS.has(text[position])) {
      // Keep the punctuation with the first sentence.
      cutIndex = position + 1;
      break;
    }
  }
  // A punctuation mark sitting at the very end leaves nothing for the second
  // half; fall back to the character midpoint in that case.
  if (cutIndex <= 0 || cutIndex >= text.length) {
    cutIndex = midpoint;
  }

  const firstText = text.slice(0, cutIndex).trim();
  const secondText = text.slice(cutIndex).trim();
  if (firstText.length === 0 || secondText.length === 0) {
    return segments;
  }

  const span = segment.endMs - segment.startMs;
  const splitMs = Math.round(segment.startMs + span * (cutIndex / text.length));
  const first: RecordingSegment = {
    text: firstText,
    startMs: segment.startMs,
    endMs: splitMs,
  };
  const second: RecordingSegment = {
    text: secondText,
    startMs: splitMs,
    endMs: segment.endMs,
  };
  return [...segments.slice(0, index), first, second, ...segments.slice(index + 1)];
}

// The translation that already exists for a mined sentence: the positionally
// paired line the viewer shows beside it. Returns null (mine the text alone,
// never generate a fresh translation) when there is no translation document, or
// when the row was merged/split — an edit shifts the row out of alignment with
// the translation's lines, so the pairing can no longer be trusted.
function pairedTranslationFor(
  index: number,
  segment: RecordingSegment,
  transcript: RecordingTextDocument | null,
  translation: RecordingTextDocument | null,
): string | null {
  if (!translation || translation.missing) {
    return null;
  }
  const original = transcript?.segments[index];
  if (
    !original ||
    original.startMs !== segment.startMs ||
    original.endMs !== segment.endMs ||
    original.text !== segment.text
  ) {
    return null;
  }
  const line = splitTranscriptSegments(translation.text)[index]?.trim();
  return line && line.length > 0 ? line : null;
}

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
  onReTranscribe,
  isReTranscribing,
  onReTranslate,
  isReTranslating,
  onMineSegment,
  isMining,
  expressionFieldMapped,
  ankiReachable,
}: {
  recording: RecentRecording;
  onBack: () => void;
  // Force a re-transcribe of this recording for the active language so an older
  // transcript can be backfilled with timestamps. Undefined disables the
  // affordance entirely.
  onReTranscribe: ((force: boolean) => void) | undefined;
  isReTranscribing: boolean;
  // Force a re-translate of this recording (overwrites the existing translation).
  // Undefined disables the affordance.
  onReTranslate: ((force: boolean) => void) | undefined;
  isReTranslating: boolean;
  // Mine a single sentence into its own Anki card. Resolves true when a card was
  // actually created, so the row can show a persistent "✓ Mined" marker. The
  // paired translation line (or null when the recording has none) rides along so
  // mining reuses the existing translation instead of generating a fresh one.
  onMineSegment: (
    text: string,
    startMs: number,
    endMs: number,
    translation: string | null,
  ) => Promise<boolean>;
  isMining: boolean;
  // Whether the Anki expression field is mapped and Anki is reachable. Together
  // they decide whether Mine is enabled and which tooltip explains a disabled one.
  expressionFieldMapped: boolean;
  ankiReachable: boolean;
}) {
  // The segments sidecar path is folded in so backfilling timestamps on an
  // already-transcribed language (same count, same translation) still changes
  // the signature and triggers a re-read once the sidecar lands.
  const changeSignature = `${recording.transcripts
    .map((transcript) => `${transcript.language}:${transcript.segmentsPath ?? ""}`)
    .join("|")}:${recording.translationPath ?? ""}`;
  const { data, status, error, reload } = useRecordingTexts({
    filePath: recording.filePath,
    changeSignature,
  });

  // Whole-file playback for this recording, driven by the compact top bar.
  // Gated on audioDeleted below so a transcript-only entry never tries to load.
  const player = useAudioPlayer();
  const isActiveTrack = player.filePath === recording.filePath;
  const handleTogglePlayback = () => {
    if (isActiveTrack) {
      player.toggle();
    } else {
      player.playRecording(recording);
    }
  };
  const handleSeekPlayback = (ms: number) => {
    if (isActiveTrack) {
      player.seekMs(ms);
    } else {
      // Nothing loaded yet — start the track so the scrub has audio to move.
      player.playRecording(recording);
    }
  };
  // Per-sentence playback rides the same player as the top bar. Disabled when
  // the local audio is gone, so timed rows still show their timestamp but no
  // play control rather than pretending playback works.
  const handlePlaySegment = recording.audioDeleted
    ? undefined
    : (startMs: number, endMs: number) =>
        player.playSegment(recording, startMs, endMs);
  const activeSegment = isActiveTrack ? player.activeSegment : null;

  const transcripts = data?.transcripts ?? [];
  const translations = data?.translations ?? [];

  const [activeLanguage, setActiveLanguage] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<TranscriptViewMode>("sideBySide");
  const [query, setQuery] = useState("");
  const [selectedSegment, setSelectedSegment] = useState<string | null>(null);
  // Links transcript row i to translation row i by POSITION only. Today's
  // translation is a whole-document translation, so row i on one side is not
  // guaranteed to be the semantic counterpart of row i on the other — the
  // pairing is purely positional. Exact per-line alignment arrives with
  // per-segment translation; there is no semantic matching here.
  const [activeSegmentIndex, setActiveSegmentIndex] = useState<number | null>(
    null,
  );

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

  // A local, in-session editable copy of the active transcript's timed segments.
  // Merge/split rewrite this copy only; nothing is persisted, and switching
  // language or reloading the transcript resets it from the source segments.
  const [editedSegments, setEditedSegments] = useState<RecordingSegment[]>([]);
  // Rows the user has mined this session, tracked by content key so the marker
  // survives re-renders but not a merge/split (which makes a new sentence).
  const [minedKeys, setMinedKeys] = useState<Set<string>>(new Set());
  // The single row with a mine request in flight, so only it shows "Mining…".
  const [miningKey, setMiningKey] = useState<string | null>(null);

  useEffect(() => {
    setEditedSegments(activeTranscript?.segments ?? []);
    setMinedKeys(new Set());
    setMiningKey(null);
  }, [activeTranscript]);

  const handleMergeSegment = (index: number) => {
    const joiner = isCjkDocument(activeTranscript) ? "" : " ";
    setEditedSegments((segments) => mergeSegmentAt(segments, index, joiner));
  };

  const handleSplitSegment = (index: number) => {
    setEditedSegments((segments) => splitSegmentAt(segments, index));
  };

  const handleMineSegment = (index: number) => {
    const segment = editedSegments[index];
    if (!segment || miningKey !== null) {
      return;
    }
    const key = segmentMineKey(segment);
    setMiningKey(key);
    const translation = pairedTranslationFor(
      index,
      segment,
      activeTranscript,
      activeTranslation,
    );
    void onMineSegment(segment.text, segment.startMs, segment.endMs, translation)
      .then((mined) => {
        if (mined) {
          setMinedKeys((previous) => {
            const next = new Set(previous);
            next.add(key);
            return next;
          });
        }
      })
      .finally(() => {
        setMiningKey((current) => (current === key ? null : current));
      });
  };

  // Mining writes an Anki card with the sentence audio, so it needs local audio
  // present. When it isn't usable, an explanatory tooltip replaces the action.
  const mineDisabledReason = !expressionFieldMapped
    ? "Map an Anki note first"
    : !ankiReachable
      ? "Anki not reachable"
      : null;

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

  // An older transcript with text but no timed segments can be backfilled by a
  // forced re-transcribe. Gated on local audio existing (nothing to re-run
  // without it) and on the transcript view being visible.
  const canEnablePerSentence =
    onReTranscribe !== undefined &&
    !recording.audioDeleted &&
    viewMode !== "translation" &&
    activeTranscript !== null &&
    !activeTranscript.missing &&
    activeTranscript.text.trim().length > 0 &&
    activeTranscript.segments.length === 0;

  // Re-run the (whole-document) translation, overwriting the existing sidecar.
  // Sits beside the re-transcribe action in the same bar.
  const canReTranslate =
    onReTranslate !== undefined && recording.translationPath !== null;

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

      {recording.audioDeleted ? (
        <p className="now-playing-unavailable">
          Local audio was deleted — playback is unavailable for this recording.
        </p>
      ) : (
        <NowPlayingBar
          variant="compact"
          fileName={recording.fileName}
          isPlaying={isActiveTrack && player.isPlaying}
          currentTimeMs={isActiveTrack ? player.currentTimeMs : 0}
          durationMs={
            isActiveTrack && player.durationMs > 0
              ? player.durationMs
              : recording.durationMs
          }
          onToggle={handleTogglePlayback}
          onSeek={handleSeekPlayback}
        />
      )}

      {canEnablePerSentence || canReTranslate ? (
        <div className="transcript-enable-timing">
          <span className="transcript-enable-timing-text">
            {canEnablePerSentence
              ? "Enable per-sentence playback — re-transcribe with timestamps."
              : "Re-run the translation for this recording."}
          </span>
          <div className="transcript-enable-timing-buttons">
            {canEnablePerSentence ? (
              <button
                type="button"
                className="transcript-enable-timing-action"
                onClick={() => onReTranscribe?.(true)}
                disabled={isReTranscribing}
              >
                {isReTranscribing ? "Re-transcribing…" : "Re-transcribe"}
              </button>
            ) : null}
            {canReTranslate ? (
              <button
                type="button"
                className="transcript-enable-timing-action"
                onClick={() => onReTranslate?.(true)}
                disabled={isReTranslating}
              >
                {isReTranslating ? "Re-translating…" : "Re-translate"}
              </button>
            ) : null}
          </div>
        </div>
      ) : null}

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
              activeSegmentIndex={activeSegmentIndex}
              onActivateSegment={setActiveSegmentIndex}
              activeSegment={activeSegment}
              onPlaySegment={handlePlaySegment}
              editable
              segmentsOverride={editedSegments}
              onMergeSegment={handleMergeSegment}
              onSplitSegment={handleSplitSegment}
              onMineSegment={
                recording.audioDeleted ? undefined : handleMineSegment
              }
              minedKeys={minedKeys}
              miningKey={miningKey}
              isMining={isMining}
              mineDisabledReason={mineDisabledReason}
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
              activeSegmentIndex={activeSegmentIndex}
              onActivateSegment={setActiveSegmentIndex}
              activeSegment={null}
              onPlaySegment={undefined}
            />
          ) : null}
        </div>
      )}
    </div>
  );
}
