import type { ActiveSegment } from "../../hooks/useAudioPlayer";
import type { RecordingTextDocument } from "../../types";
import { TranscriptSegmentRow } from "./TranscriptSegmentRow";
import { splitTranscriptSegments } from "./transcriptText";

// A row is either a timed sentence from the segments sidecar or an untimed line
// split from the plain text. Untimed rows carry null timing and never play.
type ReadingRow = {
  text: string;
  startMs: number | null;
  endMs: number | null;
};

function buildRows(document: RecordingTextDocument): ReadingRow[] {
  if (document.segments.length > 0) {
    return document.segments.map((segment) => ({
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
  missingLabel,
  selectedSegment,
  onSelectSegment,
  activeSegmentIndex,
  onActivateSegment,
  activeSegment,
  onPlaySegment,
}: {
  paneKey: string;
  kicker: string;
  title: string;
  note: string | null;
  isCjk: boolean;
  document: RecordingTextDocument | null;
  query: string;
  emptyLabel: string;
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
}) {
  const rows = document ? buildRows(document) : [];

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
          <p className="transcript-pane-empty">{emptyLabel}</p>
        ) : (
          rows.map((row, index) => {
            const key = `${paneKey}-${index}`;
            const playing =
              activeSegment !== null &&
              row.startMs !== null &&
              row.endMs !== null &&
              row.startMs === activeSegment.startMs &&
              row.endMs === activeSegment.endMs;
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
              />
            );
          })
        )}
      </div>
    </section>
  );
}
