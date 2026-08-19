import { formatDuration } from "../../lib/format";
import type { RecorderPhase } from "../../types";
import { TooltipBadge } from "../ui/Tooltip";

// A phase with no label here falls back to `statusText`, which both callers also put in the
// element's `title`. That fallback is why an unlabelled phase printed a raw filesystem path as
// the visible status during a download.
const RECORDER_STATUS_LABEL: Record<string, string> = {
  idle: "Ready",
  recording: "Recording",
  saving: "Saving",
  transcribing: "Transcribing",
  "downloading-model": "Downloading",
};

export function recorderStatusLabel(phase: RecorderPhase, statusText: string): string {
  return RECORDER_STATUS_LABEL[phase] ?? statusText;
}

export function RecorderPage({
  elapsedMs,
  phase,
  statusText,
  hotkeyTooltip,
  recorderBusy,
  isRecording,
  stopBusy,
  anyBusy,
  onStartRecording,
  onStopRecording,
  onHideToTray,
}: {
  elapsedMs: number;
  phase: RecorderPhase;
  statusText: string;
  hotkeyTooltip: string;
  recorderBusy: boolean;
  isRecording: boolean;
  stopBusy: boolean;
  anyBusy: boolean;
  onStartRecording: () => void;
  onStopRecording: () => void;
  onHideToTray: () => void;
}) {
  return (
    <div className="recorder-view">
      <article className="panel panel-primary">
        <header className="panel-header">
          <div>
            <p className="panel-kicker">Recorder</p>
            <h2>System Audio</h2>
          </div>
          <div className="panel-actions">
            <TooltipBadge label="Shortcuts" description={hotkeyTooltip} />
          </div>
        </header>

        <div className="recorder-topline">
          <div className="timer-block">
            <span className="hint-label">Elapsed</span>
            <strong>{formatDuration(elapsedMs)}</strong>
          </div>
          <div className="status-stack" title={statusText}>
            <span className="hint-label">Status</span>
            <strong>{recorderStatusLabel(phase, statusText)}</strong>
          </div>
        </div>

        <div className="action-row">
          <button type="button" onClick={onStartRecording} disabled={recorderBusy}>
            Start Recording
          </button>
          <button
            type="button"
            className="secondary"
            onClick={onStopRecording}
            disabled={!isRecording || stopBusy}
          >
            Stop Recording
          </button>
          <button
            type="button"
            className="ghost"
            onClick={onHideToTray}
            disabled={anyBusy}
          >
            Hide To Tray
          </button>
        </div>
      </article>
    </div>
  );
}
