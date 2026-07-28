import { useEffect, useRef, useState } from "react";
import { TooltipBadge } from "../ui/Tooltip";

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
    // The badge sits outside the label on purpose. A label forwards its clicks to the
    // control it wraps, so a badge inside it would focus the input — which puts the field
    // into editing mode and stops it mirroring mpv, just from reading the hint.
    <div className="watch-offset">
      <label className="watch-offset-field">
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
      </label>
      <TooltipBadge
        label="?"
        description="Positive shows the subtitles later, negative earlier. 0 clears it."
      />
    </div>
  );
}
