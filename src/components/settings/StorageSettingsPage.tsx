import type {
  AppBootstrap,
  BusyAction,
  WhisperAssetUpdateResult,
} from "../../types";
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
  const ffmpegReady = bootstrap.ffmpegDetection.status === "ready";
  // Re-downloading the recommended yt-dlp overwrites the binary in place, so the download
  // action doubles as the install for an update the check turned up.
  const ytdlpUpdateVersion =
    ytdlpUpdateResult?.status === "available"
      ? ytdlpUpdateResult.latestVersion
      : null;

  const progress = (kind: "ffmpeg" | "ytdlp" | "alass") => (
    <DownloadProgressCard
      snapshot={bootstrap.modelDownload}
      kind={kind}
      downloadIsActive={downloadIsActive}
      onTogglePause={() => void onToggleDownloadPause()}
      onCancel={() => void onCancelDownload()}
    />
  );

  return (
    <>
      <CapabilitySection
        title="MP3 Compression"
        toolName="FFmpeg"
        ready={ffmpegReady}
        callToAction={bootstrap.ffmpegDetection.message}
        description={
          "Recordings are kept as WAV while they are transcribed. Once a recording has a " +
          "transcript you can convert it to MP3 from the Library, and cards already pushed " +
          "to Anki keep working. Turn on manual MP3 conversion in App Preferences to see " +
          "the action."
        }
        action={
          ffmpegReady ? null : (
            <button
              type="button"
              onClick={() => void onDownloadRecommendedFfmpeg()}
              disabled={downloadIsActive || busyAction === "downloadFfmpeg"}
            >
              Download
            </button>
          )
        }
      >
        {progress("ffmpeg")}
      </CapabilitySection>

      <CapabilitySection
        title="YouTube Import"
        toolName="yt-dlp"
        ready={ytdlpReady}
        callToAction={bootstrap.ytdlpDetection.message}
        description={
          "Paste a YouTube link on the Home page and its audio lands in your Library, ready " +
          "to transcribe like any other recording."
        }
        onCheck={ytdlpReady ? () => void onCheckYtdlpUpdate() : undefined}
        checkBusy={busyAction === "checkYtdlpUpdate"}
        checkResult={ytdlpUpdateResult}
        action={
          ytdlpReady ? (
            // Only once a check has actually found something newer. The install and the
            // result that justifies it used to sit two sections apart.
            ytdlpUpdateVersion ? (
              <button
                type="button"
                onClick={() => void onDownloadRecommendedYtdlp()}
                disabled={downloadIsActive || busyAction === "downloadYtdlp"}
              >
                Update to {ytdlpUpdateVersion}
              </button>
            ) : null
          ) : (
            <button
              type="button"
              onClick={() => void onDownloadRecommendedYtdlp()}
              disabled={downloadIsActive || busyAction === "downloadYtdlp"}
            >
              Download
            </button>
          )
        }
      >
        {progress("ytdlp")}
      </CapabilitySection>

      {/* No check: the download URL is pinned to one tested release on purpose — see the
          comment on ALASS_RELEASE_DOWNLOAD_URL — so a newer version upstream would be
          something we could report and not act on. */}
      <CapabilitySection
        title="Subtitle Sync"
        toolName="alass"
        ready={alassReady}
        missingLabel="Optional"
        callToAction={bootstrap.alassDetection.message}
        description={
          "Realigns a subtitle file against the video, for when the timing drifts as the " +
          "episode goes on. If your subtitles are simply late or early throughout, the " +
          "offset on the Watch page fixes that on its own."
        }
        action={
          alassReady ? null : (
            <button
              type="button"
              onClick={() => void onDownloadRecommendedAlass()}
              disabled={downloadIsActive || busyAction === "downloadAlass"}
            >
              Download
            </button>
          )
        }
      >
        {progress("alass")}
      </CapabilitySection>
    </>
  );
}
