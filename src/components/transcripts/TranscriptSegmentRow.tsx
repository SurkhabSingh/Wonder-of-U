import { useState, type MouseEvent } from "react";
import { formatDuration } from "../../lib/format";
import { highlightMatches } from "./transcriptText";

export function TranscriptSegmentRow({
  segmentKey,
  text,
  query,
  selected,
  linked,
  startMs,
  endMs,
  playing,
  onPlaySegment,
  onSelect,
  onActivate,
  onDeactivate,
  editable = false,
  onMine,
  mined = false,
  mineBusy = false,
  mineDisabled = false,
  mineDisabledReason = null,
  onMerge,
  canMerge = false,
  onSplit,
  canSplit = false,
}: {
  segmentKey: string;
  text: string;
  query: string;
  selected: boolean;
  linked: boolean;
  // Timing is present only for rows built from the segments sidecar. Untimed
  // rows (older recordings, translations) leave these null and keep the
  // placeholder dot with no play control.
  startMs: number | null;
  endMs: number | null;
  playing: boolean;
  onPlaySegment: ((startMs: number, endMs: number) => void) | undefined;
  onSelect: () => void;
  onActivate: () => void;
  onDeactivate: () => void;
  // Sentence-mining + merge/split controls. Only timed transcript rows are
  // editable; when false none of the controls below render.
  editable?: boolean;
  // Undefined when mining is unavailable (local audio deleted). When present but
  // `mineDisabled`, the button is inert and explains itself via the tooltip.
  onMine?: () => void;
  mined?: boolean;
  mineBusy?: boolean;
  mineDisabled?: boolean;
  mineDisabledReason?: string | null;
  onMerge?: () => void;
  canMerge?: boolean;
  onSplit?: () => void;
  canSplit?: boolean;
}) {
  const [copied, setCopied] = useState(false);
  const hasTiming = startMs !== null && endMs !== null;
  const canPlay = hasTiming && onPlaySegment !== undefined;

  async function copySegment(event: MouseEvent<HTMLButtonElement>) {
    // The copy control lives inside a selectable row; don't toggle the row's
    // selection when the user only meant to copy the line.
    event.stopPropagation();
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    } catch {
      // Clipboard access can be denied. Leave the label untouched rather than
      // reporting a copy that did not happen.
    }
  }

  function playSegment(event: MouseEvent<HTMLButtonElement>) {
    // Same as copy: keep a play tap from also selecting the row.
    event.stopPropagation();
    if (startMs !== null && endMs !== null) {
      onPlaySegment?.(startMs, endMs);
    }
  }

  return (
    <div
      className={`transcript-segment ${selected ? "is-selected" : ""} ${
        linked ? "is-linked" : ""
      } ${playing ? "is-playing" : ""}`}
      data-segment={segmentKey}
      onClick={onSelect}
      onMouseEnter={onActivate}
      onMouseLeave={onDeactivate}
    >
      <span
        className={`transcript-segment-gutter ${hasTiming ? "has-timing" : ""}`}
      >
        {canPlay ? (
          <button
            type="button"
            className="transcript-segment-play"
            onClick={playSegment}
            aria-label={playing ? "Playing this line" : "Play this line"}
            aria-pressed={playing}
            title="Play this line"
          >
            <span aria-hidden="true">{"▶"}</span>
          </button>
        ) : (
          <span className="transcript-segment-dot" aria-hidden="true" />
        )}
        {hasTiming ? (
          <span className="transcript-segment-time">
            {formatDuration(startMs)}
          </span>
        ) : null}
      </span>
      <p className="transcript-segment-body">{highlightMatches(text, query)}</p>
      <div className="transcript-segment-aside">
        <button
          type="button"
          className="transcript-segment-copy"
          onClick={copySegment}
          title="Copy this line"
        >
          {copied ? "Copied" : "Copy"}
        </button>
        {editable ? (
          <>
            {onMerge ? (
              <button
                type="button"
                className="transcript-segment-edit"
                onClick={(event) => {
                  event.stopPropagation();
                  onMerge();
                }}
                disabled={!canMerge}
                title="Merge with the next line"
                aria-label="Merge with the next line"
              >
                <span aria-hidden="true">{"⤓"}</span>
              </button>
            ) : null}
            {onSplit ? (
              <button
                type="button"
                className="transcript-segment-edit"
                onClick={(event) => {
                  event.stopPropagation();
                  onSplit();
                }}
                disabled={!canSplit}
                title="Split this line in two"
                aria-label="Split this line in two"
              >
                <span aria-hidden="true">{"⤒"}</span>
              </button>
            ) : null}
            {onMine ? (
              mined ? (
                <span
                  className="transcript-segment-mined"
                  title="Mined to Anki"
                >
                  <span aria-hidden="true">✓</span> Mined
                </span>
              ) : (
                <button
                  type="button"
                  className="transcript-segment-mine"
                  onClick={(event) => {
                    event.stopPropagation();
                    onMine();
                  }}
                  disabled={mineDisabled || mineBusy}
                  title={mineDisabledReason ?? "Mine this sentence to Anki"}
                  aria-label="Mine this sentence to Anki"
                >
                  {mineBusy ? "Mining…" : "Mine"}
                </button>
              )
            ) : null}
          </>
        ) : (
          <span className="transcript-segment-slot" aria-hidden="true" />
        )}
      </div>
    </div>
  );
}
