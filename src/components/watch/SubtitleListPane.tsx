import { useEffect, useRef } from "react";
import { TranscriptSegmentRow } from "../transcripts/TranscriptSegmentRow";
import { ScannableText } from "./ScannableText";
import { segmentMineKey } from "../../lib/segments";
import type { RecordingSegment } from "../../types";

// The whole subtitle file as a mineable list, tracking mpv's clock.
//
// Deliberately a separate component from `TranscriptReadingPane` rather than a
// generalisation of it. That pane is used by the verified transcript viewer, and it
// identifies the playing row by EXACT start/end equality — which is only safe because
// the audio player hands back the very segment object the row was built from. A player's
// clock gives a position instead, and after a merge or split no row's bounds match a cue
// any more. Rewriting the pane to suit both would have put the verified viewer at risk
// for no gain; the expensive part (`TranscriptSegmentRow`) is reused verbatim, and the
// semantics that must not diverge — the mine key, merge, split — are shared functions.
export function SubtitleListPane({
  cues,
  positionMs,
  minedKeys,
  deckMinedKeys,
  miningKey,
  isMining,
  mineDisabledReason,
  selectedKey,
  onSelect,
  onSeek,
  onMine,
  onMerge,
  onSplit,
  scanHint,
}: {
  cues: RecordingSegment[];
  /// Where mpv is now. The playing row is found by containment, not identity.
  positionMs: number | null;
  minedKeys: Set<string>;
  deckMinedKeys: Set<string>;
  miningKey: string | null;
  isMining: boolean;
  mineDisabledReason: string | null;
  selectedKey: string | null;
  onSelect: (key: string | null) => void;
  onSeek: (startMs: number) => void;
  onMine: (index: number) => void;
  onMerge: (index: number) => void;
  onSplit: (index: number) => void;
  /// How to scan, in the user's configured terms. Passed in rather than derived here so
  /// the pane stays presentational.
  scanHint: string;
}) {
  const bodyRef = useRef<HTMLDivElement | null>(null);
  const playingIndex =
    positionMs === null
      ? -1
      : cues.findIndex(
          (cue) => positionMs >= cue.startMs && positionMs < cue.endMs,
        );

  // Follow the playing line, but only while the reader is already looking at it.
  // Scrolling unconditionally would drag them back every few seconds and make it
  // impossible to read ahead or look back — the same guard the live transcript uses.
  useEffect(() => {
    if (playingIndex < 0) {
      return;
    }
    const body = bodyRef.current;
    const row = body?.querySelector(`[data-segment="cue-${playingIndex}"]`);
    if (!body || !row) {
      return;
    }
    const bodyBox = body.getBoundingClientRect();
    const rowBox = row.getBoundingClientRect();
    // Off-screen by more than a screenful means the user has scrolled away on purpose.
    const isNearby =
      rowBox.bottom > bodyBox.top - bodyBox.height &&
      rowBox.top < bodyBox.bottom + bodyBox.height;
    if (isNearby) {
      row.scrollIntoView({ block: "nearest" });
    }
  }, [playingIndex]);

  if (cues.length === 0) {
    return (
      <div className="transcript-pane">
        <div className="transcript-pane-body">
          <p className="microcopy">
            No subtitles loaded. Pick a subtitle file when you open the video, or use one
            with a built-in subtitle track.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="transcript-pane">
      <header className="transcript-pane-header">
        <div>
          <p className="panel-kicker">Subtitles</p>
          <h3>{cues.length} lines</h3>
        </div>
        {/* The dictionary lives in the Anki add-on, so Anki has to be running. Saying so
            up front beats letting the first lookup be the thing that explains it. */}
        <span className="transcript-pane-note">
          {scanHint} &middot; &#9654; jumps the video there
        </span>
      </header>
      <div className="transcript-pane-body" ref={bodyRef}>
        {cues.map((cue, index) => {
          const key = `cue-${index}`;
          const mineKey = segmentMineKey(cue);
          return (
            <TranscriptSegmentRow
              key={key}
              segmentKey={key}
              text={cue.text}
              query=""
              selected={selectedKey === key}
              linked={false}
              startMs={cue.startMs}
              endMs={cue.endMs}
              playing={index === playingIndex}
              // The play control seeks the real player rather than starting a second
              // one — there is only ever one thing making sound.
              onPlaySegment={(startMs) => onSeek(startMs)}
              onSelect={() => onSelect(key)}
              onActivate={() => {}}
              onDeactivate={() => {}}
              editable
              onMine={() => onMine(index)}
              mined={minedKeys.has(mineKey)}
              minedInDeck={deckMinedKeys.has(mineKey)}
              mineBusy={miningKey === mineKey}
              mineDisabled={mineDisabledReason !== null || isMining}
              mineDisabledReason={mineDisabledReason}
              onMerge={() => onMerge(index)}
              canMerge={index < cues.length - 1}
              onSplit={() => onSplit(index)}
              canSplit={cue.text.length >= 2}
              renderText={(text) => <ScannableText ownerKey={key} text={text} />}
            />
          );
        })}
      </div>
    </div>
  );
}
