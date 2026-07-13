import type { RecordingTextDocument } from "../../types";
import { TranscriptSegmentRow } from "./TranscriptSegmentRow";
import { splitTranscriptSegments } from "./transcriptText";

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
}) {
  const segments = document ? splitTranscriptSegments(document.text) : [];

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
        ) : segments.length === 0 ? (
          <p className="transcript-pane-empty">{emptyLabel}</p>
        ) : (
          segments.map((segment, index) => {
            const key = `${paneKey}-${index}`;
            return (
              <TranscriptSegmentRow
                key={key}
                segmentKey={key}
                text={segment}
                query={query}
                selected={selectedSegment === key}
                onSelect={() =>
                  onSelectSegment(selectedSegment === key ? null : key)
                }
              />
            );
          })
        )}
      </div>
    </section>
  );
}
