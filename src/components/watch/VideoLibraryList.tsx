import { convertFileSrc } from "@tauri-apps/api/core";
import * as DropdownMenuPrimitive from "@radix-ui/react-dropdown-menu";
import type { SubtitleOrigin, WatchedVideo } from "../../types";
import { fileNameFromPath, formatDuration, formatTimestamp } from "../../lib/format";

// The chip a video's subtitle state draws. An origin the backend does not recognise arrives as
// null and lands on "Subtitles mapped", which is the honest reading: there IS a subtitle, we
// just have nothing to say about where it came from.
function subtitleChip(video: WatchedVideo): { label: string; tone: string } {
  if (!video.subtitlePath) {
    return { label: "No subtitles", tone: "warning" };
  }
  const byOrigin: Partial<Record<SubtitleOrigin, { label: string; tone: string }>> = {
    generated: { label: "Generated", tone: "accent" },
    synced: { label: "Realigned", tone: "accent" },
  };
  return (
    (video.subtitleOrigin ? byOrigin[video.subtitleOrigin] : undefined) ?? {
      label: "Subtitles mapped",
      tone: "success",
    }
  );
}

function VideoThumbnail({ video }: { video: WatchedVideo }) {
  if (!video.thumbnailPath) {
    return <div className="video-thumb video-thumb-empty" aria-hidden="true" />;
  }
  return (
    <img
      className="video-thumb"
      // Tauri serves local files through its own protocol; a bare path is blocked. The
      // thumbnails folder is allowed into the asset scope at startup for exactly this.
      src={convertFileSrc(video.thumbnailPath)}
      alt=""
      loading="lazy"
    />
  );
}

export function VideoLibraryList({
  videos,
  onOpen,
  onChooseSubtitle,
  onSearchJimaku,
  onGenerateSubtitles,
  onRealign,
  onForget,
  missingPaths,
  hasJimakuKey,
  generatingPath,
  generateProgress,
  onCancelGenerate,
  openMenuPath,
  onOpenMenuChange,
  isStarting,
  searchQuery,
}: {
  videos: WatchedVideo[];
  onOpen: (video: WatchedVideo) => void;
  onChooseSubtitle: (video: WatchedVideo) => void;
  onSearchJimaku: (video: WatchedVideo) => void;
  onGenerateSubtitles: (video: WatchedVideo) => void;
  onRealign: (video: WatchedVideo) => void;
  onForget: (video: WatchedVideo) => void;
  // Videos whose file could not be found. Listed and dimmed rather than hidden: the row
  // carries the subtitle pairing, and a disconnected drive should not cost it.
  missingPaths: ReadonlySet<string>;
  hasJimakuKey: boolean;
  // The video currently being transcribed, if any — its row shows the bar in place of its
  // metadata line, so progress is attached to the video it belongs to.
  generatingPath: string | null;
  generateProgress: number | null;
  onCancelGenerate: () => void;
  // Lifted so only one row's menu is open at a time, exactly as the audio rows do it.
  openMenuPath: string | null;
  onOpenMenuChange: (videoPath: string | null) => void;
  isStarting: boolean;
  searchQuery: string;
}) {
  if (videos.length === 0) {
    return (
      <p className="empty-state">
        {searchQuery.trim() ? "No videos match that search" : "No videos yet"}
        {searchQuery.trim() ? null : (
          <span className="empty-state-hint">
            Add one to keep it here with whatever subtitles you pair with it.
          </span>
        )}
      </p>
    );
  }

  return (
    <div className="video-list">
      {videos.map((video) => {
        const chip = subtitleChip(video);
        const missing = missingPaths.has(video.videoPath);
        const generating = video.videoPath === generatingPath;

        return (
          <article
            key={video.videoPath}
            className={`video-item${missing ? " is-missing" : ""}`}
          >
            <VideoThumbnail video={video} />

            <div className="video-item-text">
              <strong className="video-name" title={video.videoPath}>
                {video.title ?? fileNameFromPath(video.videoPath)}
              </strong>

              {generating ? (
                <div className="video-progress">
                  <span className="video-progress-text">
                    {generateProgress !== null
                      ? `Writing subtitles… ${generateProgress}%`
                      : "Preparing…"}
                  </span>
                  <div
                    className="video-progress-bar"
                    role="progressbar"
                    aria-label="Subtitle generation progress"
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={generateProgress ?? undefined}
                  >
                    <div className="progress-track" aria-hidden="true">
                      <div
                        className="progress-fill"
                        style={{ width: `${generateProgress ?? 0}%` }}
                      />
                    </div>
                  </div>
                  <button
                    type="button"
                    className="ghost video-progress-cancel"
                    onClick={onCancelGenerate}
                  >
                    Cancel
                  </button>
                </div>
              ) : (
                <>
                  <span className="recording-meta">
                    {formatDuration(video.durationMs)}
                    {video.lastOpenedAtMs
                      ? ` · opened ${formatTimestamp(video.lastOpenedAtMs)}`
                      : " · never opened"}
                  </span>
                  <div className="recording-state-row">
                    <span
                      className={`status-chip status-chip-${
                        missing ? "error" : chip.tone
                      }`}
                    >
                      {missing ? "File missing" : chip.label}
                    </span>
                    {video.subtitlePath && !missing ? (
                      <span
                        className="video-subtitle-name"
                        title={video.subtitlePath}
                      >
                        {fileNameFromPath(video.subtitlePath)}
                      </span>
                    ) : null}
                  </div>
                </>
              )}
            </div>

            <div className="recording-actions">
              <button
                type="button"
                className="secondary recording-primary-action"
                onClick={() => onOpen(video)}
                disabled={missing || generating || isStarting}
                title={missing ? "This file is no longer where it was." : undefined}
              >
                {isStarting ? "Opening…" : "Open in mpv"}
              </button>
              <DropdownMenuPrimitive.Root
                modal={false}
                open={openMenuPath === video.videoPath}
                onOpenChange={(nextOpen) =>
                  onOpenMenuChange(nextOpen ? video.videoPath : null)
                }
              >
                <DropdownMenuPrimitive.Trigger asChild>
                  <button
                    type="button"
                    className="secondary recording-overflow-trigger"
                    aria-label="More actions"
                    title="More actions"
                  >
                    <span aria-hidden="true">⋯</span>
                  </button>
                </DropdownMenuPrimitive.Trigger>
                <DropdownMenuPrimitive.Portal>
                  <DropdownMenuPrimitive.Content
                    className="action-menu-content"
                    align="end"
                    sideOffset={6}
                  >
                    <DropdownMenuPrimitive.Label className="action-menu-label">
                      Subtitles
                    </DropdownMenuPrimitive.Label>
                    <DropdownMenuPrimitive.Item
                      className="action-menu-item"
                      onSelect={() => onChooseSubtitle(video)}
                    >
                      Choose a file…
                    </DropdownMenuPrimitive.Item>
                    <DropdownMenuPrimitive.Item
                      className="action-menu-item"
                      onSelect={() => onSearchJimaku(video)}
                      disabled={!hasJimakuKey}
                    >
                      Search Jimaku…
                      {hasJimakuKey ? null : (
                        <span className="action-menu-meta">Needs a key</span>
                      )}
                    </DropdownMenuPrimitive.Item>
                    <DropdownMenuPrimitive.Item
                      className="action-menu-item"
                      onSelect={() => onGenerateSubtitles(video)}
                      disabled={generatingPath !== null || missing}
                    >
                      Generate subtitles
                      {generatingPath !== null ? (
                        <span className="action-menu-meta">Running</span>
                      ) : null}
                    </DropdownMenuPrimitive.Item>
                    <DropdownMenuPrimitive.Item
                      className="action-menu-item"
                      onSelect={() => onRealign(video)}
                      disabled={!video.subtitlePath || missing}
                    >
                      Realign with the audio
                      {video.subtitlePath ? null : (
                        <span className="action-menu-meta">No subtitles</span>
                      )}
                    </DropdownMenuPrimitive.Item>
                    <DropdownMenuPrimitive.Separator className="action-menu-separator" />
                    <DropdownMenuPrimitive.Item
                      className="action-menu-item action-menu-item-danger"
                      onSelect={() => onForget(video)}
                    >
                      Remove from list
                    </DropdownMenuPrimitive.Item>
                  </DropdownMenuPrimitive.Content>
                </DropdownMenuPrimitive.Portal>
              </DropdownMenuPrimitive.Root>
            </div>
          </article>
        );
      })}
    </div>
  );
}
