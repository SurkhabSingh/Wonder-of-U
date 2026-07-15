import { fileNameFromPath } from "../../lib/format";
import type {
  AppBootstrap,
  BusyAction,
  WhisperAssetUpdateResult,
} from "../../types";
import { UpdateResultCard } from "../ui/UpdateResultCard";
import { DownloadProgressCard } from "./DownloadProgressCard";

export function StorageSettingsPage({
  bootstrap,
  busyAction,
  downloadIsActive,
  ytdlpUpdateResult,
  onCancelDownload,
  onDownloadRecommendedFfmpeg,
  onDownloadRecommendedYtdlp,
  onCheckYtdlpUpdate,
  onToggleDownloadPause,
}: {
  bootstrap: AppBootstrap;
  busyAction: BusyAction;
  downloadIsActive: boolean;
  ytdlpUpdateResult: WhisperAssetUpdateResult | null;
  onCancelDownload: () => void | Promise<void>;
  onDownloadRecommendedFfmpeg: () => void | Promise<void>;
  onDownloadRecommendedYtdlp: () => void | Promise<void>;
  onCheckYtdlpUpdate: () => void | Promise<void>;
  onToggleDownloadPause: () => void | Promise<void>;
}) {
  const ytdlpReady = bootstrap.ytdlpDetection.status === "ready";
  return (
    <>
      <header className="panel-header">
        <div>
          <p className="panel-kicker">Storage</p>
          <h2>MP3 Compression</h2>
        </div>
        <span
          className={`status-chip status-chip-${
            bootstrap.ffmpegDetection.status === "ready" ? "success" : "warning"
          }`}
          title={bootstrap.ffmpegDetection.message}
        >
          {bootstrap.ffmpegDetection.status === "ready" ? "Ready" : "Missing"}
        </span>
      </header>

      <div
        className={`update-card ${
          bootstrap.ffmpegDetection.status === "ready" ? "current" : "available"
        }`}
      >
        <strong>{bootstrap.ffmpegDetection.message}</strong>
        <p className="microcopy">
          Wonder of U keeps WAV audio for transcription because that is the safest
          input path for Whisper. After a transcript exists, you can convert
          individual recordings, selected recordings, or all available WAV
          recordings to MP3 from the Library. If a card was already pushed to
          Anki, converting the local file later will not break that existing Anki
          card because Anki keeps its own copied media file. The Convert to MP3
          action stays hidden until you enable manual MP3 conversion in App
          Preferences.
        </p>
        {bootstrap.ffmpegDetection.executablePath ? (
          <p className="path-copy" title={bootstrap.ffmpegDetection.executablePath}>
            {fileNameFromPath(bootstrap.ffmpegDetection.executablePath)}
          </p>
        ) : null}
      </div>

      {bootstrap.ffmpegDetection.status !== "ready" ? (
        <div className="action-row inline-actions">
          <button
            type="button"
            onClick={() => void onDownloadRecommendedFfmpeg()}
            disabled={downloadIsActive || busyAction === "downloadFfmpeg"}
          >
            Download FFmpeg
          </button>
        </div>
      ) : null}
      <DownloadProgressCard
        snapshot={bootstrap.modelDownload}
        kind="ffmpeg"
        downloadIsActive={downloadIsActive}
        onTogglePause={() => void onToggleDownloadPause()}
        onCancel={() => void onCancelDownload()}
      />

      <header className="panel-header">
        <div>
          <p className="panel-kicker">Storage</p>
          <h2>YouTube Import</h2>
        </div>
        <span
          className={`status-chip status-chip-${ytdlpReady ? "success" : "warning"}`}
          title={bootstrap.ytdlpDetection.message}
        >
          {ytdlpReady ? "Ready" : "Missing"}
        </span>
      </header>

      <div className={`update-card ${ytdlpReady ? "current" : "available"}`}>
        <strong>
          {bootstrap.ytdlpDetection.message ||
            (ytdlpReady
              ? "yt-dlp is installed and ready to fetch YouTube audio."
              : "Install yt-dlp to import audio from a YouTube link.")}
        </strong>
        <p className="microcopy">
          Wonder of U uses yt-dlp to fetch a YouTube video's audio into your
          Library. Once it lands, transcribe it from the Library like any other
          recording. yt-dlp is fetched from its official releases (GPLv3); it is
          not bundled.
        </p>
        {bootstrap.ytdlpDetection.executablePath ? (
          <p className="path-copy" title={bootstrap.ytdlpDetection.executablePath}>
            {fileNameFromPath(bootstrap.ytdlpDetection.executablePath)}
          </p>
        ) : null}
      </div>

      <div className="action-row inline-actions">
        {ytdlpReady ? (
          <button
            type="button"
            className="secondary"
            onClick={() => void onCheckYtdlpUpdate()}
            disabled={busyAction === "checkYtdlpUpdate"}
          >
            Update yt-dlp
          </button>
        ) : (
          <button
            type="button"
            onClick={() => void onDownloadRecommendedYtdlp()}
            disabled={downloadIsActive || busyAction === "downloadYtdlp"}
          >
            Download yt-dlp
          </button>
        )}
      </div>
      <UpdateResultCard result={ytdlpUpdateResult} />
      <DownloadProgressCard
        snapshot={bootstrap.modelDownload}
        kind="ytdlp"
        downloadIsActive={downloadIsActive}
        onTogglePause={() => void onToggleDownloadPause()}
        onCancel={() => void onCancelDownload()}
      />
    </>
  );
}
