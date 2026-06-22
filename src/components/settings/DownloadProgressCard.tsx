import { fileNameFromPath, formatProgressBytes } from "../../lib/format";
import type { ModelDownloadSnapshot } from "../../types";

export function DownloadProgressCard({
  snapshot,
  kind,
  downloadIsActive,
  onTogglePause,
  onCancel,
}: {
  snapshot: ModelDownloadSnapshot;
  kind: "runtime" | "model" | "ffmpeg";
  downloadIsActive: boolean;
  onTogglePause: () => void;
  onCancel: () => void;
}) {
  if (snapshot.kind !== kind) {
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
