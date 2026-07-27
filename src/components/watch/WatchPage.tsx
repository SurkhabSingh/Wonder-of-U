import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { formatDuration } from "../../lib/format";
import { fileNameFromPath } from "../../lib/format";
import type { WatchSnapshot } from "../../types";

// Containers mpv plays that the app's own webview cannot — which is the whole reason the
// video is handed to mpv instead of being rendered here.
const VIDEO_EXTENSIONS = ["mkv", "mp4", "webm", "mov", "m4v", "avi", "ts", "flv"];
const SUBTITLE_EXTENSIONS = ["srt", "ass", "ssa", "vtt", "sub"];

export function WatchPage({
  snapshot,
  isStarting,
  error,
  onStart,
  onStop,
  onMine,
  isMining,
  mineResult,
}: {
  snapshot: WatchSnapshot;
  isStarting: boolean;
  error: string | null;
  onStart: (videoPath: string, subtitlePath: string | null) => void;
  onStop: () => void;
  onMine: () => void;
  isMining: boolean;
  // What the last mine said — success or the reason it failed. Shown rather than
  // swallowed, so a card that did not get made is never mistaken for one that did.
  mineResult: { ok: boolean; message: string } | null;
}) {
  const [videoPath, setVideoPath] = useState<string | null>(null);
  const [subtitlePath, setSubtitlePath] = useState<string | null>(null);

  const pickVideo = async () => {
    const picked = await open({
      multiple: false,
      filters: [{ name: "Video", extensions: VIDEO_EXTENSIONS }],
    });
    if (typeof picked === "string") {
      setVideoPath(picked);
    }
  };

  const pickSubtitle = async () => {
    const picked = await open({
      multiple: false,
      filters: [{ name: "Subtitles", extensions: SUBTITLE_EXTENSIONS }],
    });
    if (typeof picked === "string") {
      setSubtitlePath(picked);
    }
  };

  return (
    <>
      <header className="panel-header">
        <div>
          <p className="panel-kicker">Watch &amp; Mine</p>
          <h2>{snapshot.connected ? snapshot.title ?? "Playing" : "Open a video"}</h2>
        </div>
        {snapshot.connected ? (
          <div className="panel-actions">
            <button type="button" className="secondary" onClick={onStop}>
              Stop
            </button>
          </div>
        ) : null}
      </header>

      {snapshot.connected ? (
        <>
          <div className="watch-transport">
            <span className="watch-clock">
              {formatDuration(snapshot.positionMs ?? 0)}
              {snapshot.durationMs ? ` / ${formatDuration(snapshot.durationMs)}` : ""}
            </span>
            <span
              className={`status-chip status-chip-${snapshot.paused ? "warning" : "success"}`}
            >
              {snapshot.paused ? "Paused" : "Playing"}
            </span>
          </div>

          {/* The line mpv currently has on screen, with the bounds it reports. This is
              exactly what a mine will use, so showing it is also how the user confirms
              the app and the player agree before trusting the hotkey. */}
          <div className="watch-line">
            {snapshot.subtitleText ? (
              <>
                <p className="watch-line-text">{snapshot.subtitleText}</p>
                <p className="microcopy">
                  {formatDuration(snapshot.subtitleStartMs ?? 0)} &ndash;{" "}
                  {formatDuration(snapshot.subtitleEndMs ?? 0)}
                  {snapshot.subtitleDelayMs !== 0
                    ? ` · offset ${snapshot.subtitleDelayMs > 0 ? "+" : ""}${snapshot.subtitleDelayMs}ms`
                    : ""}
                </p>
              </>
            ) : (
              <p className="microcopy">
                No subtitle on screen right now. If none ever appears, the video may have
                no subtitles loaded &mdash; pick a subtitle file and open it again.
              </p>
            )}
          </div>

          <div className="panel-actions watch-actions">
            <button
              type="button"
              onClick={onMine}
              disabled={isMining || !snapshot.subtitleText}
              title={
                snapshot.subtitleText
                  ? "Make an Anki card from this line"
                  : "There is no line on screen to mine"
              }
            >
              {isMining ? "Mining…" : "Mine this line"}
            </button>
          </div>

          {mineResult ? (
            <div className={`update-card ${mineResult.ok ? "current" : "error"}`}>
              <strong>{mineResult.message}</strong>
            </div>
          ) : null}
        </>
      ) : (
        <>
          <div className="info-note">
            <p className="microcopy">
              The video plays in <strong>mpv</strong>, not in this window &mdash; mpv
              handles MKV, H.265 and everything else this app&rsquo;s built-in player
              cannot. Wonder of U reads the position and the on-screen subtitle line from
              it, so you can mine the line you are hearing.
            </p>
          </div>

          <div className="settings-grid">
            <label className="field">
              <span>Video</span>
              <button type="button" className="secondary" onClick={() => void pickVideo()}>
                {videoPath ? fileNameFromPath(videoPath) : "Choose a video…"}
              </button>
            </label>

            <label className="field">
              <span>Subtitles (optional)</span>
              <button
                type="button"
                className="secondary"
                onClick={() => void pickSubtitle()}
              >
                {subtitlePath ? fileNameFromPath(subtitlePath) : "Choose a subtitle file…"}
              </button>
            </label>
          </div>

          <p className="microcopy">
            Leave subtitles empty if the file already has them built in &mdash; mpv reads
            embedded tracks on its own.
          </p>

          <div className="panel-actions">
            <button
              type="button"
              onClick={() => videoPath && onStart(videoPath, subtitlePath)}
              disabled={!videoPath || isStarting}
            >
              {isStarting ? "Opening…" : "Open in mpv"}
            </button>
          </div>

          {error ? (
            <div className="update-card error">
              <strong>{error}</strong>
            </div>
          ) : null}
        </>
      )}
    </>
  );
}
