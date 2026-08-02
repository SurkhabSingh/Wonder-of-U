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

// `segmentsOverride` lets the transcript pane render from an in-session edited
// copy of the timed segments (merge/split) instead of the document's own. When
// absent or empty, rows fall back to the document's segments, then to untimed
// lines split from the plain text.
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
}: {
  paneKey: string;
  kicker: string;
  title: string;
  note: string | null;
  isCjk: boolean;
  document: RecordingTextDocument | null;
  query: string;
  // Shown when there is no transcript document at all (never transcribed).
  emptyLabel: string;
  // Shown when a transcript document exists but carries no text — i.e. transcription
  // ran and the recording had no detectable speech (a silent capture). Defaults to
  // `emptyLabel` for panes (like translation) where the distinction does not apply.
  noSpeechLabel?: string;
  missingLabel: string;
  selectedSegment: string | null;
  onSelectSegment: (key: string | null) => void;
  // Positional link shared across both panes: the row at this index is
  // highlighted in every pane that has one. See TranscriptViewerPage for why
  // this pairing is positional rather than semantic.
  activeSegmentIndex: number | null;
  onActivateSegment: (index: number | null) => void;
  // The segment currently playing through the shared viewer player, used to
  // light up the matching timed row. `undefined` onPlaySegment means playback
  // is unavailable (e.g. the local audio was deleted).
  activeSegment: ActiveSegment | null;
  onPlaySegment: ((startMs: number, endMs: number) => void) | undefined;
  // Sentence-mining + merge/split affordances, only wired for the transcript
  // pane. When `editable` is false (the translation pane), none of these render.
  editable?: boolean;
  segmentsOverride?: RecordingSegment[];
  // Undefined when mining is unavailable for the recording (local audio gone);
  // the Mine button is then omitted while merge/split stay available.
  onMineSegment?: (index: number) => void;
  onMergeSegment?: (index: number) => void;
  onSplitSegment?: (index: number) => void;
  // Content keys of rows mined during this session (their Mine button is spent), of
  // rows whose sentence already exists in the deck (flagged but still mineable), and
  // the row currently mining. Any in-flight mine disables the other rows' Mine buttons.
  minedKeys?: Set<string>;
  deckMinedKeys?: Set<string>;
  miningKey?: string | null;
  isMining?: boolean;
  // Non-null when Anki isn't usable; becomes the disabled Mine button's tooltip.
  mineDisabledReason?: string | null;
  // How new each line is. Built from the same `buildRows` call the viewer makes, so
  // entry N describes row N; null when nothing has ranked this pane.
  ranking?: TranscriptRanking | null;
  // Narrows the pane to the lines a single word away. Rows are hidden, never
  // reordered — the order IS the recording, and reading along while listening
  // depends on it.
  withinReachOnly?: boolean;
  // Why a batch mine could not make a card of a row, keyed the same way mined rows
  // are. Keyed rather than indexed so a merge or split drops the marker with the
  // sentence it belonged to instead of moving it onto a neighbour.
  mineFailures?: Map<string, string>;
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
            // Filtered rows are skipped rather than removed from the list, so
            // `index` stays the index the sidecar, the mine action and the paired
            // pane all agree on.
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
            // Merge/split edit only timed rows; the untimed fallback stays plain.
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
                // A mine in flight elsewhere blocks a second concurrent request.
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
              />
            );
          })
        )}
      </div>
    </section>
  );
}
