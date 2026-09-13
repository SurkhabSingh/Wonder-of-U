import { useRecordingLevel } from "../../hooks/useRecordingLevel";

const DISPLAY_GAMMA = 0.6;

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
