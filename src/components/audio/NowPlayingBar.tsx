import { formatDuration } from "../../lib/format";

type NowPlayingBarVariant = "dock" | "compact";

export function NowPlayingBar({
  variant = "dock",
  fileName,
  isPlaying,
  currentTimeMs,
  durationMs,
  onToggle,
  onSeek,
  onStop,
}: {
  variant?: NowPlayingBarVariant;
  fileName: string;
  isPlaying: boolean;
  currentTimeMs: number;
  durationMs: number;
  onToggle: () => void;
  onSeek: (ms: number) => void;
  // Omitted for the compact viewer bar, which has no close affordance.
  onStop?: () => void;
}) {
  // The range needs a non-zero max; before real metadata arrives the current
  // position still gives us a floor so the thumb never sits past the end.
  const total = durationMs > 0 ? durationMs : 0;
  const max = total > 0 ? total : Math.max(currentTimeMs, 1);
  const clampedCurrent = Math.min(Math.max(currentTimeMs, 0), max);
  const fillPercent = max > 0 ? (clampedCurrent / max) * 100 : 0;

  return (
    <div
      className={`now-playing-bar now-playing-bar-${variant}`}
      role="group"
      aria-label="Audio player"
    >
      <button
        type="button"
        className="now-playing-toggle"
        onClick={onToggle}
        aria-label={isPlaying ? "Pause" : "Play"}
      >
        <span aria-hidden="true">{isPlaying ? "❚❚" : "▶"}</span>
      </button>

      {variant === "dock" ? (
        <span className="now-playing-name" title={fileName}>
          {fileName}
        </span>
      ) : null}

      <input
        type="range"
        className="now-playing-seek"
        min={0}
        max={max}
        step={100}
        value={clampedCurrent}
        onChange={(event) => onSeek(Number(event.target.value))}
        style={{ ["--seek-fill" as string]: `${fillPercent}%` }}
        aria-label={`Seek${fileName ? ` ${fileName}` : ""}`}
        aria-valuetext={`${formatDuration(clampedCurrent)} of ${formatDuration(total)}`}
      />

      <span className="now-playing-time">
        {formatDuration(clampedCurrent)} / {formatDuration(total)}
      </span>

      {onStop ? (
        <button
          type="button"
          className="now-playing-close"
          onClick={onStop}
          aria-label="Stop and close player"
        >
          <span aria-hidden="true">✕</span>
        </button>
      ) : null}
    </div>
  );
}
