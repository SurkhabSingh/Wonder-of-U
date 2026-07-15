import { useCallback, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { IMPORT_MEDIA_EXTENSIONS } from "../../constants";
import { formatDuration, formatTimestamp } from "../../lib/format";
import {
  filterSupportedMediaPaths,
  normalizeSelections,
  recordingChips,
} from "../../lib/helpers";
import { useFileDrop } from "../../hooks/useFileDrop";
import type { RecentRecording, RecorderPhase } from "../../types";
import { recorderStatusLabel } from "../recorder/RecorderPage";
import { TooltipBadge } from "../ui/Tooltip";

const SUPPORTED_FORMATS_HINT = IMPORT_MEDIA_EXTENSIONS.join(", ");

export function HomePage({
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
  recentRecordings,
  needsTranscriptCount,
  needsTranslationCount,
  readyForAnkiCount,
  transcriptionLanguage,
  recordingPushedToCurrentAnkiDeck,
  isImporting,
  onImportMedia,
  onView,
  onOpenLibrary,
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
  recentRecordings: RecentRecording[];
  needsTranscriptCount: number;
  needsTranslationCount: number;
  readyForAnkiCount: number;
  transcriptionLanguage: string;
  recordingPushedToCurrentAnkiDeck: (recording: RecentRecording) => boolean;
  isImporting: boolean;
  onImportMedia: (paths: string[]) => void;
  onView: (filePath: string) => void;
  onOpenLibrary: () => void;
}) {
  const recent = recentRecordings.slice(0, 5);
  // A rejected drop or a failed picker has to say so. Silently doing nothing is
  // the one outcome the drop zone must never have.
  const [importNote, setImportNote] = useState<string | null>(null);
  // An import must not be queued behind another one, and no import should start
  // while the recorder or another batch job is mid-flight.
  const importDisabled = isImporting || anyBusy;

  const handleDroppedPaths = useCallback(
    (paths: string[]) => {
      const supported = filterSupportedMediaPaths(paths);

      if (supported.length === 0) {
        setImportNote(
          paths.length === 1
            ? "That file is not a supported audio or video format."
            : "None of those files are a supported audio or video format.",
        );
        return;
      }

      const rejectedCount = paths.length - supported.length;
      setImportNote(
        rejectedCount > 0
          ? `${rejectedCount} unsupported file${
              rejectedCount === 1 ? " was" : "s were"
            } skipped.`
          : null,
      );
      onImportMedia(supported);
    },
    [onImportMedia],
  );

  const { isDraggingOver } = useFileDrop({
    enabled: !importDisabled,
    onDrop: handleDroppedPaths,
  });

  const handleBrowse = useCallback(async () => {
    try {
      const paths = normalizeSelections(
        await open({
          multiple: true,
          filters: [
            {
              name: "Audio & video",
              // Same list the drop filter gates on — they cannot drift.
              extensions: [...IMPORT_MEDIA_EXTENSIONS],
            },
          ],
        }),
      );

      if (paths.length === 0) {
        return;
      }

      handleDroppedPaths(paths);
    } catch {
      setImportNote("The file chooser could not be opened.");
    }
  }, [handleDroppedPaths]);

  return (
    <div className="home-view">
      <article className="panel home-record-card">
        <div className="home-record-info">
          <p className="panel-kicker">Capture</p>
          <h2>Record system audio</h2>
          <div className="home-record-status">
            <span className="home-record-elapsed">{formatDuration(elapsedMs)}</span>
            <span className="home-record-phase" title={statusText}>
              {recorderStatusLabel(phase, statusText)}
            </span>
          </div>
        </div>
        <div className="home-record-actions">
          <TooltipBadge label="Shortcuts" description={hotkeyTooltip} />
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

      <div className="home-needs-row">
        <button type="button" className="home-stat" onClick={onOpenLibrary}>
          <span className="home-stat-value is-warning">{needsTranscriptCount}</span>
          <span className="home-stat-label">Need transcript</span>
        </button>
        <button type="button" className="home-stat" onClick={onOpenLibrary}>
          <span className="home-stat-value is-accent">{needsTranslationCount}</span>
          <span className="home-stat-label">Need translation</span>
        </button>
        <button type="button" className="home-stat" onClick={onOpenLibrary}>
          <span className="home-stat-value is-success">{readyForAnkiCount}</span>
          <span className="home-stat-label">Ready for Anki</span>
        </button>
      </div>

      <article className="panel home-recent">
        <header className="panel-header">
          <div>
            <p className="panel-kicker">Recent</p>
            <h2>Latest recordings</h2>
          </div>
          {recentRecordings.length > 0 ? (
            <button type="button" className="ghost" onClick={onOpenLibrary}>
              View library
            </button>
          ) : null}
        </header>

        {recent.length === 0 ? (
          <p className="empty-state">No recordings yet</p>
        ) : (
          <ul className="home-recent-list">
            {recent.map((recording) => {
              const canReadTranscript =
                recording.transcripts.length > 0 ||
                recording.transcriptPath !== null;
              const chips = recordingChips(
                recording,
                transcriptionLanguage,
                recordingPushedToCurrentAnkiDeck,
              );

              return (
                <li className="home-recent-item" key={recording.filePath}>
                  <div className="home-recent-main">
                    {canReadTranscript ? (
                      <button
                        type="button"
                        className="recording-filename-button"
                        onClick={() => onView(recording.filePath)}
                        title="Read transcript and translation"
                      >
                        {recording.fileName}
                      </button>
                    ) : (
                      <strong className="home-recent-name">
                        {recording.fileName}
                      </strong>
                    )}
                    <span className="home-recent-meta">
                      {formatDuration(recording.durationMs)} ·{" "}
                      {formatTimestamp(recording.createdAtMs)}
                    </span>
                  </div>
                  {chips.length > 0 ? (
                    <div className="home-recent-chips">
                      {chips.map((chip) => (
                        <span
                          key={chip.label}
                          className={`home-chip home-chip-${chip.tone}`}
                        >
                          {chip.label}
                        </span>
                      ))}
                    </div>
                  ) : null}
                </li>
              );
            })}
          </ul>
        )}
      </article>

      <div
        className={`home-dropzone${
          isDraggingOver ? " is-dragging" : ""
        }${importDisabled ? " is-busy" : ""}`}
      >
        <span className="home-dropzone-icon" aria-hidden="true">
          ＋
        </span>
        <span className="home-dropzone-label">
          {isImporting
            ? "Importing files…"
            : isDraggingOver
              ? "Drop to import"
              : "Drop audio or video to import"}
        </span>
        <button
          type="button"
          className="secondary home-dropzone-browse"
          onClick={() => void handleBrowse()}
          disabled={importDisabled}
        >
          Browse…
        </button>
        {importNote ? (
          <small className="home-dropzone-note">{importNote}</small>
        ) : (
          <small>Supported: {SUPPORTED_FORMATS_HINT}</small>
        )}
      </div>
    </div>
  );
}
