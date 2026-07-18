import { useRecordingLevel } from "../../hooks/useRecordingLevel";

// The raw peak is a linear amplitude, where quiet dialogue barely moves the bar.
// A gentle gamma lifts low signals into visible territory without pinning loud
// ones to full — the meter's whole job is to answer "is audio actually being
// captured?", so faint speech still has to register.
const DISPLAY_GAMMA = 0.6;

/**
 * A slim horizontal VU-style meter for the live recording input level. Renders a
 * green→amber→red gradient that a clip-path reveals from the left in proportion
 * to the current peak, so the color at any fill width stays fixed. Reads 0 (and
 * an empty bar) whenever `active` is false.
 */
export function RecordingLevelMeter({ active }: { active: boolean }) {
  const level = useRecordingLevel(active);
  const pct = active
    ? Math.min(100, Math.round(Math.pow(Math.max(0, level), DISPLAY_GAMMA) * 100))
    : 0;

  return (
    <div
      className={`level-meter${active ? " is-active" : ""}`}
      role="meter"
      aria-label="Recording input level"
      aria-valuenow={pct}
      aria-valuemin={0}
      aria-valuemax={100}
    >
      <div
        className="level-meter-fill"
        style={{ clipPath: `inset(0 ${100 - pct}% 0 0)` }}
      />
    </div>
  );
}
