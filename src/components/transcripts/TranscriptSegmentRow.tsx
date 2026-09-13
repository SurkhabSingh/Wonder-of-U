import { useState, type MouseEvent } from "react";
import { formatDuration } from "../../lib/format";
import type { LineRanking } from "../../types";
import { isBareWord, isWithinReach } from "../../hooks/useSentenceRanking";
import { ScannableText } from "../scanner/ScannableText";
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
  ranking = null,
  mineFailure = null,
  activeMatchOccurrence = null,
}: {
  segmentKey: string;
  text: string;
  query: string;
  selected: boolean;
  linked: boolean;
  startMs: number | null;
  endMs: number | null;
  playing: boolean;
  onPlaySegment: ((startMs: number, endMs: number) => void) | undefined;
  onSelect: () => void;
  onActivate: () => void;
  onDeactivate: () => void;
  editable?: boolean;
  onMine?: () => void;
  mined?: boolean;
  minedInDeck?: boolean;
  mineBusy?: boolean;
  mineDisabled?: boolean;
  mineDisabledReason?: string | null;
  onMerge?: () => void;
  canMerge?: boolean;
  onSplit?: () => void;
  canSplit?: boolean;
  ranking?: LineRanking | null;
  mineFailure?: string | null;
  activeMatchOccurrence?: number | null;
}) {
  const [copied, setCopied] = useState(false);
  const hasTiming = startMs !== null && endMs !== null;
  const canPlay = hasTiming && onPlaySegment !== undefined;

  async function copySegment(event: MouseEvent<HTMLButtonElement>) {
    event.stopPropagation();
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    } catch {}
  }

  function playSegment(event: MouseEvent<HTMLButtonElement>) {
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
        {ranking && ranking.contentWordCount > 0 ? (
          <span
            className={`transcript-segment-newness ${
              isBareWord(ranking)
                ? "is-bare-word"
                : isWithinReach(ranking)
                  ? "is-within-reach"
                  : ranking.unknownWords.length === 0
                    ? "is-known"
                    : ""
            }`}
            title={
              ranking.unknownWords.length === 0
                ? "You know every word in this line."
                : isBareWord(ranking)
                  ? `New here: ${ranking.unknownWords.join(
                      "、",
                    )} — but this line has nothing else in it to learn the word from.`
                  : `New here: ${ranking.unknownWords.join("、")}`
            }
          >
            {ranking.unknownWords.length === 0
              ? "✓"
              : `+${ranking.unknownWords.length}`}
          </span>
        ) : null}
      </span>
      <p className="transcript-segment-body">
        <ScannableText
          ownerKey={`row:${segmentKey}`}
          startMs={startMs}
          endMs={endMs}
        >
          {highlightMatches(text, query, activeMatchOccurrence)}
        </ScannableText>
      </p>
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
                {mineFailure ? (
                  <span
                    className="transcript-segment-mined is-failed"
                    title={mineFailure}
                  >
                    <span aria-hidden="true">!</span> Failed
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
