import { useEffect, useRef, useState } from "react";

// The subtitle offset, as one typeable field.
//
// mpv owns the value and the watch page re-reads it four times a second, so the field
// cannot simply be bound to it — the poll would overwrite whatever was half-typed. It
// mirrors mpv while unfocused and holds its own draft while being edited, committing on
// Enter or blur. That is also why the commit is absolute rather than a delta: the player is
// the source of truth, and sending "+100" against a stale reading would compound.
export function SubtitleOffsetField({
  delayMs,
  onCommit,
}: {
  delayMs: number;
  onCommit: (delayMs: number) => void;
}) {
  const [draft, setDraft] = useState(String(delayMs));
  const [editing, setEditing] = useState(false);
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (!editing) {
      setDraft(String(delayMs));
    }
  }, [delayMs, editing]);

  function commit() {
    setEditing(false);
    const parsed = Number(draft.trim());
    // An unparseable entry snaps back to what mpv actually has rather than guessing, so a
    // stray keystroke can never silently move the subtitles.
    if (!Number.isFinite(parsed)) {
      setDraft(String(delayMs));
      return;
    }
    const clamped = Math.round(Math.max(-600_000, Math.min(600_000, parsed)));
    setDraft(String(clamped));
    if (clamped !== delayMs) {
      onCommit(clamped);
    }
  }

  return (
    <label className="watch-offset">
      <span>Offset</span>
      <span className="watch-offset-input">
        <input
          ref={inputRef}
          type="number"
          step={50}
          value={draft}
          aria-label="Subtitle offset in milliseconds"
          onFocus={() => setEditing(true)}
          onChange={(event) => setDraft(event.currentTarget.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.currentTarget.blur();
            } else if (event.key === "Escape") {
              setEditing(false);
              setDraft(String(delayMs));
              event.currentTarget.blur();
            }
          }}
        />
        <span className="watch-offset-unit">ms</span>
      </span>
      <span className="microcopy">
        Positive shows subtitles later, negative earlier. 0 clears it.
      </span>
    </label>
  );
}
