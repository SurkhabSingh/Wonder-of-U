import { formatDuration } from "../../lib/format";
import { ThemedSelect } from "../ui/ThemedSelect";

type NowPlayingBarVariant = "dock" | "compact";

// Language-learning-friendly speeds: slow for hard audio, a little fast for review.
const SPEED_OPTIONS = [0.5, 0.75, 1, 1.25, 1.5].map((rate) => ({
  value: String(rate),
  label: `${rate}×`,
}));

export function NowPlayingBar({
  variant = "dock",
  fileName,
  isPlaying,
  currentTimeMs,
  durationMs,
  onToggle,
  onSeek,
  onStop,
  playbackRate,
  onSetPlaybackRate,
  isRepeating = false,
  onToggleRepeat,
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
  // Speed control renders only when a setter is supplied (both variants use it).
  playbackRate?: number;
  onSetPlaybackRate?: (rate: number) => void;
  // Repeat-the-sentence renders only when a handler is supplied — the per-sentence
  // viewer bar passes it; the whole-file dock bar does not.
  isRepeating?: boolean;
  onToggleRepeat?: () => void;
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

      {onToggleRepeat || onSetPlaybackRate ? (
        <div className="now-playing-options">
          {onToggleRepeat ? (
            <button
              type="button"
              className={`now-playing-repeat${isRepeating ? " is-active" : ""}`}
              onClick={onToggleRepeat}
              aria-pressed={isRepeating}
              aria-label="Repeat the current sentence"
              title="Repeat the current sentence"
            >
              <span aria-hidden="true">↻</span>
            </button>
          ) : null}

          {onSetPlaybackRate ? (
            <ThemedSelect
              value={String(playbackRate ?? 1)}
              options={SPEED_OPTIONS}
              onChange={(value) => onSetPlaybackRate(Number(value))}
              placeholder="Speed"
              title="Playback speed"
              triggerClassName="now-playing-speed-trigger"
            />
          ) : null}
        </div>
      ) : null}

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
