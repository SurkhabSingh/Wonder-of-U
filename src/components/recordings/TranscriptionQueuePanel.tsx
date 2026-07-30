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

// What became of transcriptions that have already ended, rendered above the Library list.
//
// Live work is no longer here: a running recording carries its own progress bar, Live
// transcript and Cancel on its own row, and a queued one carries Remove from queue in its
// menu. What a row cannot show is a run that has finished — above all a failure and its
// message — so that is what this keeps, plus the control to dismiss it.
export function TranscriptionQueuePanel({
  items,
  finishedCount,
  onClearFinished,
}: {
  items: TranscriptionQueueItem[];
  finishedCount: number;
  onClearFinished: () => void;
}) {
  // Live work now lives on the recording's own row — its progress, its Live transcript,
  // its Cancel, and Remove from queue in its menu. What a row cannot show is what happened
  // to a run that has already ended, especially a failure, so that is all this keeps.
  //
  // Nothing was deleted without a new home first: the three controls above moved to the
  // rows before this was narrowed.
  const finishedItems = items.filter(
    (item) => item.status !== "active" && item.status !== "queued",
  );

  // No history, no panel — a clean run leaves no empty chrome behind.
  if (finishedItems.length === 0) {
    return null;
  }

  return (
    <section className="transcription-queue" aria-label="Transcription queue">
      <div className="transcription-queue-header">
        <span className="transcription-queue-label">Finished transcriptions</span>
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

      <ul className="transcription-queue-list">
        {finishedItems.map((item) => {
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
              <span
                className={`status-chip ${STATUS_CHIP[item.status]}`}
                aria-label={STATUS_LABEL[item.status]}
              >
                {STATUS_LABEL[item.status].toLowerCase()}
              </span>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
