import type { TranscriptionQueueItem } from "../../types";

// Decorative status glyphs (aria-hidden); each row carries an aria-label with
// the spelled-out status for the reader. Mirrors the Home YouTube queue glyphs.
const STATUS_GLYPH: Record<TranscriptionQueueItem["status"], string> = {
  queued: "•",
  active: "⟳",
  done: "✓",
  failed: "!",
  cancelled: "–",
};

const STATUS_LABEL: Record<TranscriptionQueueItem["status"], string> = {
  queued: "Queued",
  active: "Transcribing",
  done: "Done",
  failed: "Failed",
  cancelled: "Cancelled",
};

const STATUS_CHIP: Record<TranscriptionQueueItem["status"], string> = {
  queued: "status-chip-neutral",
  active: "status-chip-accent",
  done: "status-chip-success",
  failed: "status-chip-error",
  cancelled: "status-chip-warning",
};

// The non-blocking transcription queue, rendered above the Library list. Cloned
// from the Home YouTube queue: a flat list of status rows, the one `active` row
// carrying a live progress bar + Cancel, queued rows a × remove, terminal rows a
// status chip, and a Clear finished control once there is history to drop.
export function TranscriptionQueuePanel({
  items,
  activeProgress,
  currentIndex,
  total,
  finishedCount,
  onCancel,
  onRemove,
  onClearFinished,
}: {
  items: TranscriptionQueueItem[];
  activeProgress: number | null;
  currentIndex: number;
  total: number;
  finishedCount: number;
  onCancel: () => void;
  onRemove: (id: string) => void;
  onClearFinished: () => void;
}) {
  // No rows, no panel — matches the YouTube queue, which is absent until work is
  // queued.
  if (items.length === 0) {
    return null;
  }

  const isTranscribing = items.some((item) => item.status === "active");

  return (
    <section className="transcription-queue" aria-label="Transcription queue">
      <div className="transcription-queue-header">
        <span className="transcription-queue-label">Transcription queue</span>
        {/* Finished rows are history, not work in progress — offer the dismissal
            only once there is something to dismiss. */}
        {finishedCount > 0 ? (
          <button
            type="button"
            className="ghost transcription-queue-clear"
            onClick={onClearFinished}
          >
            Clear finished
          </button>
        ) : null}
      </div>

      {isTranscribing && total > 0 ? (
        <p className="transcription-queue-progress">
          Transcribing {currentIndex} of {total}…
        </p>
      ) : null}

      <ul className="transcription-queue-list">
        {items.map((item) => {
          const label = item.title ?? item.filePath;
          return (
            <li className="transcription-queue-item" key={item.id}>
              <span className="transcription-queue-glyph" aria-hidden="true">
                {STATUS_GLYPH[item.status]}
              </span>
              <span
                className="transcription-queue-title"
                title={item.message ?? label}
              >
                {label}
              </span>
              {item.status === "active" ? (
                <div className="transcription-queue-active">
                  {/* Completion is the awaited invoke resolving, not this bar
                      hitting 100 — the bar is live feedback fed by the
                      transcription-progress event. */}
                  <div className="progress-track" aria-hidden="true">
                    <div
                      className="progress-fill"
                      style={{
                        width: `${Math.max(
                          0,
                          Math.min(100, activeProgress ?? 0),
                        )}%`,
                      }}
                    />
                  </div>
                  <button
                    type="button"
                    className="ghost transcription-queue-cancel"
                    onClick={onCancel}
                  >
                    Cancel
                  </button>
                </div>
              ) : (
                <span
                  className={`status-chip ${STATUS_CHIP[item.status]}`}
                  aria-label={STATUS_LABEL[item.status]}
                >
                  {STATUS_LABEL[item.status].toLowerCase()}
                </span>
              )}
              {item.status === "queued" ? (
                <button
                  type="button"
                  className="ghost transcription-queue-remove"
                  onClick={() => onRemove(item.id)}
                  aria-label="Remove from queue"
                  title="Remove from queue"
                >
                  ×
                </button>
              ) : null}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
