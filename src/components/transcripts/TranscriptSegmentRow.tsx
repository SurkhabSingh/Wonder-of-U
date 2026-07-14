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
        {/* Reserved, deliberately quiet slot for the sentence-mining action
            that grows here in a later roadmap item. */}
        <span className="transcript-segment-slot" aria-hidden="true" />
      </div>
    </div>
  );
}
