import { convertFileSrc } from "@tauri-apps/api/core";
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
    jimaku: { label: "Subtitles mapped", tone: "success" },
    picked: { label: "Subtitles mapped", tone: "success" },
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
      // Tauri serves local files through its own protocol; a bare path would be blocked.
      src={convertFileSrc(video.thumbnailPath)}
      alt=""
      loading="lazy"
    />
  );
}

export function VideoLibraryList({
  videos,
  selectedPath,
  onSelect,
  missingPaths,
}: {
  videos: WatchedVideo[];
  selectedPath: string | null;
  onSelect: (videoPath: string) => void;
  // Videos whose file could not be found. They stay listed rather than disappearing: dropping
  // the row would take the subtitle mapping with it, and a disconnected drive should not cost
  // a pairing the user spent effort on.
  missingPaths: ReadonlySet<string>;
}) {
  if (videos.length === 0) {
    return (
      <p className="empty-state">
        No videos yet
        <span className="empty-state-hint">
          Add one to keep it here with whatever subtitles you pair it with.
        </span>
      </p>
    );
  }

  return (
    <div className="video-list">
      {videos.map((video) => {
        const chip = subtitleChip(video);
        const missing = missingPaths.has(video.videoPath);
        const selected = video.videoPath === selectedPath;
        return (
          <article
            key={video.videoPath}
            className={`video-item${selected ? " is-selected" : ""}${
              missing ? " is-missing" : ""
            }`}
          >
            <button
              type="button"
              className="video-item-main"
              onClick={() => onSelect(video.videoPath)}
              aria-expanded={selected}
              title={video.videoPath}
            >
              <VideoThumbnail video={video} />
              <span className="video-item-text">
                <strong className="video-name">
                  {video.title ?? fileNameFromPath(video.videoPath)}
                </strong>
                <span className="recording-meta">
                  {formatDuration(video.durationMs)}
                  {video.lastOpenedAtMs
                    ? ` · opened ${formatTimestamp(video.lastOpenedAtMs)}`
                    : " · never opened"}
                </span>
              </span>
              <span
                className={`status-chip status-chip-${missing ? "error" : chip.tone}`}
              >
                {missing ? "File missing" : chip.label}
              </span>
            </button>
          </article>
        );
      })}
    </div>
  );
}
