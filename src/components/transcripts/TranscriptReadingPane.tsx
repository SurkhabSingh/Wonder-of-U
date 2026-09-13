import type { ActiveSegment } from "../../hooks/useAudioPlayer";
import { isWithinReach } from "../../hooks/useSentenceRanking";
import type {
  RecordingSegment,
  RecordingTextDocument,
  TranscriptRanking,
} from "../../types";
import { TranscriptSegmentRow } from "./TranscriptSegmentRow";
import { splitTranscriptSegments } from "./transcriptText";

// A row is either a timed sentence from the segments sidecar or an untimed line
// split from the plain text. Untimed rows carry null timing and never play.
type ReadingRow = {
  text: string;
  startMs: number | null;
  endMs: number | null;
};

export function buildRows(
  document: RecordingTextDocument,
  segmentsOverride: RecordingSegment[] | undefined,
): ReadingRow[] {
  const segments =
    segmentsOverride && segmentsOverride.length > 0
      ? segmentsOverride
      : document.segments;
  if (segments.length > 0) {
    return segments.map((segment) => ({
      text: segment.text,
      startMs: segment.startMs,
      endMs: segment.endMs,
    }));
  }
  return splitTranscriptSegments(document.text).map((text) => ({
    text,
    startMs: null,
    endMs: null,
  }));
}

export function TranscriptReadingPane({
  paneKey,
  kicker,
  title,
  note,
  isCjk,
  document,
  query,
  emptyLabel,
  noSpeechLabel = emptyLabel,
  missingLabel,
  selectedSegment,
  onSelectSegment,
  activeSegmentIndex,
  onActivateSegment,
  activeSegment,
  onPlaySegment,
  editable = false,
  segmentsOverride,
  onMineSegment,
  onMergeSegment,
  onSplitSegment,
  minedKeys,
  deckMinedKeys,
  miningKey = null,
  isMining = false,
  mineDisabledReason = null,
  ranking = null,
  withinReachOnly = false,
  mineFailures,
  activeMatch = null,
}: {
  paneKey: string;
  kicker: string;
  title: string;
  note: string | null;
  isCjk: boolean;
  document: RecordingTextDocument | null;
  query: string;
  emptyLabel: string;
  noSpeechLabel?: string;
  missingLabel: string;
  selectedSegment: string | null;
  onSelectSegment: (key: string | null) => void;
  activeSegmentIndex: number | null;
  onActivateSegment: (index: number | null) => void;
  activeSegment: ActiveSegment | null;
  onPlaySegment: ((startMs: number, endMs: number) => void) | undefined;
  editable?: boolean;
  segmentsOverride?: RecordingSegment[];
  onMineSegment?: (index: number) => void;
  onMergeSegment?: (index: number) => void;
  onSplitSegment?: (index: number) => void;
  minedKeys?: Set<string>;
  deckMinedKeys?: Set<string>;
  miningKey?: string | null;
  isMining?: boolean;
  mineDisabledReason?: string | null;
  ranking?: TranscriptRanking | null;
  withinReachOnly?: boolean;
  mineFailures?: Map<string, string>;
  activeMatch?: { index: number; occurrence: number } | null;
}) {
  const rows = document ? buildRows(document, segmentsOverride) : [];
  const hiddenByFilter =
    withinReachOnly && ranking
      ? rows.filter((_, index) => {
          const line = ranking.lines[index];
          return !line || !isWithinReach(line);
        }).length
      : 0;

  return (
    <section className={`transcript-pane ${isCjk ? "is-cjk" : ""}`}>
      <header className="transcript-pane-header">
        <div className="transcript-pane-heading">
          <p className="panel-kicker">{kicker}</p>
          <h3 className="transcript-pane-title">{title}</h3>
        </div>
        {note ? <span className="transcript-pane-note">{note}</span> : null}
      </header>
      <div className="transcript-pane-body">
        {document === null ? (
          <p className="transcript-pane-empty">{emptyLabel}</p>
        ) : document.missing ? (
          <p className="transcript-pane-missing">{missingLabel}</p>
        ) : rows.length === 0 ? (
          <p className="transcript-pane-empty">{noSpeechLabel}</p>
        ) : hiddenByFilter === rows.length ? (
          <p className="transcript-pane-empty">
            No line here is a single word away. Turn the filter off to read the
            whole transcript.
          </p>
        ) : (
          rows.map((row, index) => {
            if (withinReachOnly && ranking) {
              const line = ranking.lines[index];
              if (!line || !isWithinReach(line)) {
                return null;
              }
            }
            const key = `${paneKey}-${index}`;
            const timed = row.startMs !== null && row.endMs !== null;
            const playing =
              activeSegment !== null &&
              row.startMs !== null &&
              row.endMs !== null &&
              row.startMs === activeSegment.startMs &&
              row.endMs === activeSegment.endMs;
            const rowEditable = editable && timed;
            const mineKey = timed
              ? `${row.startMs}:${row.endMs}:${row.text}`
              : null;
            const mined =
              mineKey !== null && (minedKeys?.has(mineKey) ?? false);
            const minedInDeck =
              mineKey !== null && (deckMinedKeys?.has(mineKey) ?? false);
            const mineBusy = mineKey !== null && miningKey === mineKey;
            return (
              <TranscriptSegmentRow
                key={key}
                segmentKey={key}
                text={row.text}
                query={query}
                selected={selectedSegment === key}
                linked={activeSegmentIndex === index}
                startMs={row.startMs}
                endMs={row.endMs}
                playing={playing}
                onPlaySegment={onPlaySegment}
                onSelect={() => {
                  onActivateSegment(index);
                  onSelectSegment(selectedSegment === key ? null : key);
                }}
                onActivate={() => onActivateSegment(index)}
                onDeactivate={() => onActivateSegment(null)}
                editable={rowEditable}
                onMine={
                  rowEditable && onMineSegment
                    ? () => onMineSegment(index)
                    : undefined
                }
                mined={mined}
                minedInDeck={minedInDeck}
                mineBusy={mineBusy}
                mineDisabled={mineDisabledReason !== null || (isMining && !mineBusy)}
                mineDisabledReason={mineDisabledReason}
                onMerge={
                  rowEditable && onMergeSegment
                    ? () => onMergeSegment(index)
                    : undefined
                }
                canMerge={index < rows.length - 1}
                onSplit={
                  rowEditable && onSplitSegment
                    ? () => onSplitSegment(index)
                    : undefined
                }
                canSplit={row.text.length >= 2}
                ranking={ranking?.lines[index] ?? null}
                mineFailure={
                  mineKey !== null ? (mineFailures?.get(mineKey) ?? null) : null
                }
                activeMatchOccurrence={
                  activeMatch?.index === index ? activeMatch.occurrence : null
                }
              />
            );
          })
        )}
      </div>
    </section>
  );
}
