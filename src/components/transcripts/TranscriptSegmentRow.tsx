import { useState, type MouseEvent } from "react";
import { highlightMatches } from "./transcriptText";

export function TranscriptSegmentRow({
  segmentKey,
  text,
  query,
  selected,
  linked,
  onSelect,
  onActivate,
  onDeactivate,
}: {
  segmentKey: string;
  text: string;
  query: string;
  selected: boolean;
  linked: boolean;
  onSelect: () => void;
  onActivate: () => void;
  onDeactivate: () => void;
}) {
  const [copied, setCopied] = useState(false);

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

  return (
    <div
      className={`transcript-segment ${selected ? "is-selected" : ""} ${
        linked ? "is-linked" : ""
      }`}
      data-segment={segmentKey}
      onClick={onSelect}
      onMouseEnter={onActivate}
      onMouseLeave={onDeactivate}
    >
      <span className="transcript-segment-gutter" aria-hidden="true">
        <span className="transcript-segment-dot" />
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
