import { fileNameFromPath, formatProgressBytes } from "../../lib/format";
import type { AssetKind, ModelDownloadSnapshot } from "../../types";

export function DownloadProgressCard({
  snapshot,
  kind,
  downloadIsActive,
  onTogglePause,
  onCancel,
}: {
  snapshot: ModelDownloadSnapshot;
  // The shared AssetKind, not a union rewritten here. The local copy this replaces had
  // drifted from Rust's list, and the asset it was missing simply never showed progress.
  //
  // Optional, meaning "whatever is downloading". Settings shows six of these side by side and
  // each must show only its own asset; Home shows one and does not know — or need to know —
  // which asset the queue is on. An absent prop is the only honest way to say that: passing
  // the snapshot's own kind back in would compare a value with itself and read as a bug.
  kind?: AssetKind;
  downloadIsActive: boolean;
  onTogglePause: () => void;
  onCancel: () => void;
}) {
  if (kind !== undefined && snapshot.kind !== kind) {
    return null;
  }

  if (snapshot.status === "idle" && snapshot.targetPath === null) {
    return null;
  }

  return (
    <div className="download-card">
      <div className="progress-track" aria-hidden="true">
        <div
          className="progress-fill"
          style={{
            width: `${Math.max(0, Math.min(100, snapshot.progressPercent ?? 0))}%`,
          }}
        />
      </div>
      <p className="microcopy">
        {snapshot.message}{" "}
        {formatProgressBytes(snapshot.downloadedBytes, snapshot.totalBytes)}
        {snapshot.progressPercent !== null
          ? ` (${snapshot.progressPercent.toFixed(1)}%)`
          : ""}
        {/* Only one download runs at a time, so anything else asked for is waiting behind this
            one. Saying so is what stops a queued request looking like a press that did nothing. */}
        {snapshot.queuedRemaining > 0
          ? ` · ${snapshot.queuedRemaining} more queued`
          : ""}
      </p>
      {snapshot.targetPath ? (
        <p className="path-copy" title={snapshot.targetPath}>
          {fileNameFromPath(snapshot.targetPath)}
        </p>
      ) : null}
      {downloadIsActive ? (
        <div className="action-row compact-actions">
          <button
            type="button"
            className="secondary"
            onClick={onTogglePause}
            disabled={
              snapshot.status === "starting" || snapshot.status === "cancelling"
            }
          >
            {snapshot.status === "paused" ? "Resume Download" : "Pause Download"}
          </button>
          <button
            type="button"
            className="ghost"
            onClick={onCancel}
            disabled={snapshot.status === "cancelling"}
          >
            Cancel Download
          </button>
        </div>
      ) : null}
    </div>
  );
}
