import type {
  AppBootstrap,
  BusyAction,
  WhisperAssetUpdateResult,
} from "../../types";
import { UpdateResultCard } from "../ui/UpdateResultCard";
import { DownloadProgressCard } from "./DownloadProgressCard";
import { CapabilitySection } from "./CapabilitySection";

export function StorageSettingsPage({
  bootstrap,
  busyAction,
  downloadIsActive,
  ytdlpUpdateResult,
  onCancelDownload,
  onDownloadRecommendedFfmpeg,
  onDownloadRecommendedYtdlp,
  onDownloadRecommendedAlass,
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
  onDownloadRecommendedAlass: () => void | Promise<void>;
  onCheckYtdlpUpdate: () => void | Promise<void>;
  onToggleDownloadPause: () => void | Promise<void>;
}) {
  const ytdlpReady = bootstrap.ytdlpDetection.status === "ready";
  const alassReady = bootstrap.alassDetection.status === "ready";
  // Re-downloading the recommended yt-dlp overwrites the binary in place, so the
  // download action doubles as the install for an update the check turned up.
  const ytdlpUpdateVersion =
    ytdlpUpdateResult?.status === "available"
      ? ytdlpUpdateResult.latestVersion
      : null;
  const ffmpegReady = bootstrap.ffmpegDetection.status === "ready";
  return (
    <>
      <CapabilitySection
        title="MP3 Compression"
        ready={ffmpegReady}
        callToAction={bootstrap.ffmpegDetection.message}
        help={
          "Recordings are kept as WAV while they are transcribed. Once a recording has a " +
          "transcript you can convert it to MP3 from the Library, one at a time or in bulk, " +
          "and cards already pushed to Anki keep working. The action appears once you turn " +
          "on manual MP3 conversion in App Preferences."
        }
      >
        {ffmpegReady ? null : (
          <div className="action-row inline-actions">
            <button
              type="button"
              onClick={() => void onDownloadRecommendedFfmpeg()}
              disabled={downloadIsActive || busyAction === "downloadFfmpeg"}
            >
              Download
            </button>
          </div>
        )}
        <DownloadProgressCard
          snapshot={bootstrap.modelDownload}
          kind="ffmpeg"
          downloadIsActive={downloadIsActive}
          onTogglePause={() => void onToggleDownloadPause()}
          onCancel={() => void onCancelDownload()}
        />
      </CapabilitySection>

      <CapabilitySection
        title="YouTube Import"
        ready={ytdlpReady}
        callToAction={bootstrap.ytdlpDetection.message}
        help={
          "Paste a YouTube link on the Home page and its audio lands in your Library, ready " +
          "to transcribe like any other recording."
        }
      >
        <div className="action-row inline-actions">
          {ytdlpReady ? (
            <button
              type="button"
              className="secondary"
              onClick={() => void onCheckYtdlpUpdate()}
              disabled={busyAction === "checkYtdlpUpdate"}
            >
              Check for updates
            </button>
          ) : (
            <button
              type="button"
              onClick={() => void onDownloadRecommendedYtdlp()}
              disabled={downloadIsActive || busyAction === "downloadYtdlp"}
            >
              Download
            </button>
          )}
        </div>
        <UpdateResultCard result={ytdlpUpdateResult} />
        {/* The install for an update the check turned up. This and the progress bar below
            used to render at the very bottom of the page, under Subtitle Sync, so the
            result of checking appeared here and the button to act on it appeared two
            sections away. */}
        {ytdlpReady && ytdlpUpdateVersion ? (
          <div className="action-row compact-actions">
            <button
              type="button"
              onClick={() => void onDownloadRecommendedYtdlp()}
              disabled={downloadIsActive || busyAction === "downloadYtdlp"}
            >
              Update to {ytdlpUpdateVersion}
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
      </CapabilitySection>

      <CapabilitySection
        title="Subtitle Sync"
        ready={alassReady}
        missingLabel="Optional"
        callToAction={bootstrap.alassDetection.message}
        help={
          "Realigns a subtitle file against the video, for when the timing drifts as the " +
          "episode goes on. If your subtitles are simply late or early throughout, the " +
          "offset on the Watch page fixes that on its own."
        }
      >
        <div className="action-row inline-actions">
          <button
            type="button"
            className={alassReady ? "secondary" : undefined}
            onClick={() => void onDownloadRecommendedAlass()}
            disabled={downloadIsActive || busyAction === "downloadAlass"}
          >
            {alassReady ? "Re-download" : "Download"}
          </button>
        </div>
        <DownloadProgressCard
          snapshot={bootstrap.modelDownload}
          kind="alass"
          downloadIsActive={downloadIsActive}
          onTogglePause={() => void onToggleDownloadPause()}
          onCancel={() => void onCancelDownload()}
        />
      </CapabilitySection>
    </>
  );
}
