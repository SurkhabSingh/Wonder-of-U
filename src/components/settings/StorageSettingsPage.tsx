import { fileNameFromPath } from "../../lib/format";
import type {
  AppBootstrap,
  BusyAction,
  WhisperAssetUpdateResult,
} from "../../types";
import { isDownloadBusy } from "../../types";
import { UpdateResultCard } from "../ui/UpdateResultCard";
import { DownloadProgressCard } from "./DownloadProgressCard";

export function StorageSettingsPage({
  bootstrap,
  busyAction,
  downloadIsActive,
  ytdlpUpdateResult,
  onCancelDownload,
  onDownloadRecommendedFfmpeg,
  onReinstallFfmpeg,
  onDownloadRecommendedYtdlp,
  onDownloadRecommendedAlass,
  onDownloadRecommendedMpv,
  onReinstallMpv,
  onCheckYtdlpUpdate,
  onToggleDownloadPause,
}: {
  bootstrap: AppBootstrap;
  busyAction: BusyAction;
  downloadIsActive: boolean;
  ytdlpUpdateResult: WhisperAssetUpdateResult | null;
  onCancelDownload: () => void | Promise<void>;
  onDownloadRecommendedFfmpeg: () => void | Promise<void>;
  onReinstallFfmpeg: () => void | Promise<void>;
  onDownloadRecommendedYtdlp: () => void | Promise<void>;
  onDownloadRecommendedAlass: () => void | Promise<void>;
  onDownloadRecommendedMpv: () => void | Promise<void>;
  onReinstallMpv: () => void | Promise<void>;
  onCheckYtdlpUpdate: () => void | Promise<void>;
  onToggleDownloadPause: () => void | Promise<void>;
}) {
  const ytdlpReady = bootstrap.ytdlpDetection.status === "ready";
  const alassReady = bootstrap.alassDetection.status === "ready";
  const mpvReady = bootstrap.mpvDetection.status === "ready";
  // Re-downloading the recommended yt-dlp overwrites the binary in place, so the
  // download action doubles as the install for an update the check turned up.
  const ytdlpUpdateVersion =
    ytdlpUpdateResult?.status === "available"
      ? ytdlpUpdateResult.latestVersion
      : null;
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

      {/* Reachable in BOTH states, deliberately. Offering this only while FFmpeg was missing
          meant the app's own copy could never be replaced — no repair when it broke, and no way
          to move to a different build. The install path is the same either way; only the
          question differs, so the label is what changes.

          Gated on `managed`, not on `status`: detection reports "ready" for an FFmpeg found on
          PATH too, and there is nothing of ours to reinstall in that case. */}
      <div className="action-row inline-actions">
        {bootstrap.ffmpegDetection.managed ? (
          <button
            type="button"
            className="secondary"
            onClick={() => void onReinstallFfmpeg()}
            disabled={isDownloadBusy(busyAction)}
            title="Downloads a fresh copy and replaces the app's copy of FFmpeg"
          >
            Reinstall FFmpeg
          </button>
        ) : (
          <button
            type="button"
            onClick={() => void onDownloadRecommendedFfmpeg()}
            disabled={isDownloadBusy(busyAction)}
          >
            Download FFmpeg
          </button>
        )}
      </div>
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
            disabled={isDownloadBusy(busyAction)}
          >
            Download yt-dlp
          </button>
        )}
      </div>
      <UpdateResultCard result={ytdlpUpdateResult} />

      <header className="panel-header">
        <div>
          <p className="panel-kicker">Storage</p>
          <h2>Watch &amp; Mine Player</h2>
        </div>
        <span
          className={`status-chip status-chip-${mpvReady ? "success" : "warning"}`}
          title={bootstrap.mpvDetection.message}
        >
          {mpvReady ? "Ready" : "Missing"}
        </span>
      </header>

      <div
        className={`update-card ${mpvReady ? "current" : "available"}`}
      >
        <strong>{bootstrap.mpvDetection.message}</strong>
        <p className="microcopy">
          Watching a video and mining lines as you go needs mpv. Your own
          install is used when you have one; otherwise the app keeps its own
          copy. It downloads about 77&nbsp;MB and keeps 56&nbsp;MB, and the
          download is checked against a published fingerprint before it is
          unpacked.
        </p>
        {bootstrap.mpvDetection.executablePath ? (
          <p className="path-copy" title={bootstrap.mpvDetection.executablePath}>
            {fileNameFromPath(bootstrap.mpvDetection.executablePath)}
          </p>
        ) : null}
      </div>

      {/* Offered even when a system mpv was found, because that one is not ours to rely on:
          it can be uninstalled or upgraded to something the app cannot drive.

          The managed case sends a REINSTALL, not a download. The plain download skips when a
          runnable copy is present, so a button offering to replace one would have reported a
          download it never made. */}
      <div className="action-row inline-actions">
        {bootstrap.mpvDetection.managed ? (
          <button
            type="button"
            className="secondary"
            onClick={() => void onReinstallMpv()}
            disabled={isDownloadBusy(busyAction)}
            title="Downloads a fresh copy and replaces the app's copy of mpv"
          >
            Reinstall mpv
          </button>
        ) : (
          <button
            type="button"
            className={mpvReady ? "secondary" : undefined}
            onClick={() => void onDownloadRecommendedMpv()}
            disabled={isDownloadBusy(busyAction)}
          >
            {mpvReady ? "Download the app's own mpv" : "Download mpv"}
          </button>
        )}
      </div>
      <DownloadProgressCard
        snapshot={bootstrap.modelDownload}
        kind="mpv"
        downloadIsActive={downloadIsActive}
        onTogglePause={() => void onToggleDownloadPause()}
        onCancel={() => void onCancelDownload()}
      />

      <header className="panel-header">
        <div>
          <p className="panel-kicker">Storage</p>
          <h2>Subtitle Sync</h2>
        </div>
        <span
          className={`status-chip status-chip-${alassReady ? "success" : "warning"}`}
          title={bootstrap.alassDetection.message}
        >
          {alassReady ? "Ready" : "Optional"}
        </span>
      </header>

      <div className={`update-card ${alassReady ? "current" : "available"}`}>
        <strong>{bootstrap.alassDetection.message}</strong>
        <p className="microcopy">
          alass realigns a subtitle file against the video&rsquo;s own audio, for releases
          where the drift changes across the episode and a single offset cannot fix it. If
          your subtitles are simply late or early by a constant, the offset control on the
          Watch page is instant and needs none of this. alass is fetched from its official
          releases (GPL-3.0) and run as a separate program; only its 3.5&nbsp;MB binary is
          kept &mdash; it reuses the FFmpeg above rather than installing a second copy.
        </p>
        {bootstrap.alassDetection.executablePath ? (
          <p className="path-copy" title={bootstrap.alassDetection.executablePath}>
            {fileNameFromPath(bootstrap.alassDetection.executablePath)}
          </p>
        ) : null}
      </div>

      <div className="action-row inline-actions">
        <button
          type="button"
          className={alassReady ? "secondary" : undefined}
          onClick={() => void onDownloadRecommendedAlass()}
          disabled={isDownloadBusy(busyAction)}
        >
          {alassReady ? "Re-download alass" : "Download alass"}
        </button>
      </div>
      {/* alass had a button and no progress card, so downloading it showed nothing at all —
          and because every card checks the one shared snapshot, it hid the others too. */}
      <DownloadProgressCard
        snapshot={bootstrap.modelDownload}
        kind="alass"
        downloadIsActive={downloadIsActive}
        onTogglePause={() => void onToggleDownloadPause()}
        onCancel={() => void onCancelDownload()}
      />
      {ytdlpReady && ytdlpUpdateVersion ? (
        <div className="action-row compact-actions">
          <button
            type="button"
            onClick={() => void onDownloadRecommendedYtdlp()}
            disabled={isDownloadBusy(busyAction)}
          >
            Download {ytdlpUpdateVersion}
          </button>
        </div>
      ) : null}
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
