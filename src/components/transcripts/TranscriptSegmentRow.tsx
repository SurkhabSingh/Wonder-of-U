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
  minedInDeck = false,
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
  // Mined during THIS session: a card was just created from this exact row, so the
  // action is spent and the button goes away.
  mined?: boolean;
  // The same sentence already exists somewhere in the deck, from any past session.
  // Worth flagging, but NOT worth blocking: short lines ("はい。", "Yeah.") recur
  // across recordings, and the user may well want this one with its own audio.
  minedInDeck?: boolean;
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
              <>
                {mined || minedInDeck ? (
                  // A green "Mined" beside a live "Mine again" button would
                  // contradict itself, so the deck match gets its own quieter chip
                  // and wording: it reports a fact, it does not claim the action is
                  // finished.
                  <span
                    className={`transcript-segment-mined${
                      mined ? "" : " is-in-deck"
                    }`}
                    title={
                      mined
                        ? "Mined to Anki"
                        : "This sentence is already in your Anki deck"
                    }
                  >
                    <span aria-hidden="true">✓</span>{" "}
                    {mined ? "Mined" : "In deck"}
                  </span>
                ) : null}
                {mined ? null : (
                  <button
                    type="button"
                    className="transcript-segment-mine"
                    onClick={(event) => {
                      event.stopPropagation();
                      onMine();
                    }}
                    disabled={mineDisabled || mineBusy}
                    title={
                      mineDisabledReason ??
                      (minedInDeck
                        ? "Already in your deck — mine this line again with its own audio"
                        : "Mine this sentence to Anki")
                    }
                    aria-label={
                      minedInDeck
                        ? "Mine this sentence to Anki again"
                        : "Mine this sentence to Anki"
                    }
                  >
                    {mineBusy ? "Mining…" : minedInDeck ? "Mine again" : "Mine"}
                  </button>
                )}
              </>
            ) : null}
          </>
        ) : (
          <span className="transcript-segment-slot" aria-hidden="true" />
        )}
      </div>
    </div>
  );
}
