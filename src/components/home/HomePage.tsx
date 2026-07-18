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
import type {
  RecentRecording,
  RecorderPhase,
  RecordingFilter,
  YoutubeQueueItem,
} from "../../types";
import { recorderStatusLabel } from "../recorder/RecorderPage";
import { RecordingLevelMeter } from "../recorder/RecordingLevelMeter";
import { TooltipBadge } from "../ui/Tooltip";

const SUPPORTED_FORMATS_HINT = IMPORT_MEDIA_EXTENSIONS.join(", ");

// Unicode status glyphs for the YouTube queue rows. Decorative (aria-hidden);
// each row carries an aria-label with the spelled-out status for the reader.
const YOUTUBE_STATUS_GLYPH: Record<YoutubeQueueItem["status"], string> = {
  queued: "•",
  active: "⟳",
  done: "✓",
  failed: "!",
  cancelled: "–",
};

const YOUTUBE_STATUS_LABEL: Record<YoutubeQueueItem["status"], string> = {
  queued: "Queued",
  active: "Fetching",
  done: "Done",
  failed: "Failed",
  cancelled: "Cancelled",
};

const YOUTUBE_STATUS_CHIP: Record<YoutubeQueueItem["status"], string> = {
  queued: "status-chip-neutral",
  active: "status-chip-accent",
  done: "status-chip-success",
  failed: "status-chip-error",
  cancelled: "status-chip-warning",
};

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
  isFetchingYoutube,
  youtubeItems,
  youtubeCurrentIndex,
  youtubeTotal,
  onEnqueueYoutube,
  onRemoveYoutube,
  youtubeFinishedCount,
  onClearFinishedYoutube,
  youtubeActiveProgress,
  onCancelYoutube,
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
  isFetchingYoutube: boolean;
  youtubeItems: YoutubeQueueItem[];
  youtubeCurrentIndex: number;
  youtubeTotal: number;
  onEnqueueYoutube: (text: string) => void;
  onRemoveYoutube: (id: string) => void;
  youtubeFinishedCount: number;
  onClearFinishedYoutube: () => void;
  youtubeActiveProgress: number | null;
  onCancelYoutube: () => void | Promise<void>;
  onView: (filePath: string) => void;
  onOpenLibrary: (filter?: RecordingFilter) => void;
}) {
  const recent = recentRecordings.slice(0, 5);
  // A rejected drop or a failed picker has to say so. Silently doing nothing is
  // the one outcome the drop zone must never have.
  const [importNote, setImportNote] = useState<string | null>(null);
  // An import must not be queued behind another one, and no import should start
  // while the recorder or another batch job is mid-flight.
  const importDisabled = isImporting || anyBusy;
  const [youtubeUrl, setYoutubeUrl] = useState("");

  // Queuing is decoupled from `importDisabled` on purpose: you can keep adding
  // links while a fetch runs. The queue itself serializes the downloads.
  const handleAddYoutube = useCallback(() => {
    const trimmed = youtubeUrl.trim();
    if (trimmed.length === 0) {
      return;
    }
    onEnqueueYoutube(trimmed);
    setYoutubeUrl("");
  }, [onEnqueueYoutube, youtubeUrl]);

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
          {phase === "recording" ? (
            <RecordingLevelMeter active />
          ) : null}
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
        <button
          type="button"
          className="home-stat"
          onClick={() => onOpenLibrary("needsTranscription")}
        >
          <span className="home-stat-value is-warning">{needsTranscriptCount}</span>
          <span className="home-stat-label">Need transcript</span>
        </button>
        <button
          type="button"
          className="home-stat"
          onClick={() => onOpenLibrary("needsTranslation")}
        >
          <span className="home-stat-value is-accent">{needsTranslationCount}</span>
          <span className="home-stat-label">Need translation</span>
        </button>
        <button
          type="button"
          className="home-stat"
          onClick={() => onOpenLibrary("needsAnki")}
        >
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
            <button
              type="button"
              className="ghost"
              onClick={() => onOpenLibrary()}
            >
              View library
            </button>
          ) : null}
        </header>

        {recent.length === 0 ? (
          <div className="empty-state">
            No recordings yet
            <span className="empty-state-hint">
              Record or import to see your recordings here
            </span>
          </div>
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
          +
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

        <div className="home-youtube-row">
          <div className="home-youtube-header">
            <span className="home-youtube-label">From YouTube</span>
            {/* Finished rows are history, not work in progress — offer the
                dismissal only once there is something to dismiss. */}
            {youtubeFinishedCount > 0 ? (
              <button
                type="button"
                className="ghost home-youtube-clear"
                onClick={onClearFinishedYoutube}
              >
                Clear finished
              </button>
            ) : null}
          </div>
          <div className="input-with-action">
            {/* Input + Add stay ENABLED while a fetch runs — you can keep
                queuing links; the queue serializes the actual downloads. */}
            <input
              type="url"
              value={youtubeUrl}
              placeholder="Paste a YouTube link (or several)"
              onChange={(event) => setYoutubeUrl(event.currentTarget.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  handleAddYoutube();
                }
              }}
            />
            <button
              type="button"
              className="secondary"
              onClick={handleAddYoutube}
              disabled={youtubeUrl.trim().length === 0}
            >
              Add
            </button>
          </div>

          {isFetchingYoutube && youtubeTotal > 0 ? (
            <p className="home-youtube-progress">
              Fetching {youtubeCurrentIndex} of {youtubeTotal}…
            </p>
          ) : null}

          {youtubeItems.length > 0 ? (
            <ul className="home-youtube-queue">
              {youtubeItems.map((item) => {
                const label = item.title ?? item.url;
                return (
                  <li className="home-youtube-queue-item" key={item.id}>
                    <span
                      className="home-youtube-glyph"
                      aria-hidden="true"
                    >
                      {YOUTUBE_STATUS_GLYPH[item.status]}
                    </span>
                    <span
                      className="home-youtube-title"
                      title={item.message ?? label}
                    >
                      {label}
                    </span>
                    {item.status === "active" ? (
                      <div className="home-youtube-active">
                        {/* Completion is the awaited import resolving, not this
                            bar hitting 100 — the bar is just live feedback fed
                            by the youtube-progress event. */}
                        <div className="progress-track" aria-hidden="true">
                          <div
                            className="progress-fill"
                            style={{
                              width: `${Math.max(
                                0,
                                Math.min(100, youtubeActiveProgress ?? 0),
                              )}%`,
                            }}
                          />
                        </div>
                        <button
                          type="button"
                          className="ghost home-youtube-cancel"
                          onClick={() => void onCancelYoutube()}
                        >
                          Cancel
                        </button>
                      </div>
                    ) : (
                      <span
                        className={`status-chip ${
                          YOUTUBE_STATUS_CHIP[item.status]
                        }`}
                        aria-label={YOUTUBE_STATUS_LABEL[item.status]}
                      >
                        {YOUTUBE_STATUS_LABEL[item.status].toLowerCase()}
                      </span>
                    )}
                    {item.status === "queued" ? (
                      <button
                        type="button"
                        className="ghost home-youtube-remove"
                        onClick={() => onRemoveYoutube(item.id)}
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
          ) : null}
        </div>
      </div>
    </div>
  );
}
