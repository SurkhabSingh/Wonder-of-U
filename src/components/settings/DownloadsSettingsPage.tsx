import type {
  AppBootstrap,
  BusyAction,
  WhisperAssetUpdateResult,
} from "../../types";
import { DownloadProgressCard } from "./DownloadProgressCard";
import { DownloadRow } from "./DownloadRow";
import { SettingsDisclosure } from "./SettingsDisclosure";

// The binaries the app fetches and keeps working, as one list.
export function DownloadsSettingsPage({
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
  const ffmpeg = bootstrap.ffmpegDetection;
  const ytdlp = bootstrap.ytdlpDetection;
  const alass = bootstrap.alassDetection;
  const ffmpegReady = ffmpeg.status === "ready";
  const ytdlpReady = ytdlp.status === "ready";
  const alassReady = alass.status === "ready";

  // alass is optional, so "all set up" means the two the app actually needs plus alass if it
  // is there. Counting all three keeps the chip honest about what is on disk.
  const installed = [ffmpegReady, ytdlpReady, alassReady].filter(Boolean).length;
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
    <SettingsDisclosure
      title="Downloads"
      defaultOpen={!ffmpegReady || !ytdlpReady}
      tone={ffmpegReady && ytdlpReady ? "ready" : "attention"}
      status={
        <span
          className={`status-chip status-chip-${
            ffmpegReady && ytdlpReady ? "success" : "warning"
          }`}
        >
          {installed} of 3 installed
        </span>
      }
    >
      <div className="download-list">
        <DownloadRow
          title="MP3 compression"
          toolName="FFmpeg"
          version={ffmpeg.version}
          ready={ffmpegReady}
          description={
            ffmpegReady
              ? "Convert transcribed recordings to MP3 from the Library."
              : ffmpeg.message
          }
          action={
            // The download URL tracks a rolling build, so fetching it again IS the update.
            // No check exists because there is no version in that URL to compare against.
            <button
              type="button"
              className={ffmpegReady ? "secondary" : undefined}
              onClick={() => void onDownloadRecommendedFfmpeg()}
              disabled={downloadIsActive || busyAction === "downloadFfmpeg"}
            >
              {ffmpegReady ? "Update" : "Download"}
            </button>
          }
        >
          {progress("ffmpeg")}
        </DownloadRow>

        <DownloadRow
          title="YouTube import"
          toolName="yt-dlp"
          version={ytdlp.version}
          ready={ytdlpReady}
          note={ytdlpReady ? ytdlpUpdateResult?.message : null}
          description={
            ytdlpReady
              ? "Paste a YouTube link on the Home page to import its audio."
              : ytdlp.message
          }
          action={
            ytdlpReady ? (
              // The only one of the three that can be asked whether it is behind.
              ytdlpUpdateVersion ? (
                <button
                  type="button"
                  onClick={() => void onDownloadRecommendedYtdlp()}
                  disabled={downloadIsActive || busyAction === "downloadYtdlp"}
                >
                  Update to {ytdlpUpdateVersion}
                </button>
              ) : (
                <button
                  type="button"
                  className="secondary"
                  onClick={() => void onCheckYtdlpUpdate()}
                  disabled={busyAction === "checkYtdlpUpdate"}
                >
                  {busyAction === "checkYtdlpUpdate" ? "Checking…" : "Check"}
                </button>
              )
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
        </DownloadRow>

        <DownloadRow
          title="Subtitle sync"
          toolName="alass"
          version={alass.version}
          ready={alassReady}
          missingLabel="Optional"
          description={
            alassReady
              ? "Realign a subtitle file when its timing drifts across an episode."
              : alass.message
          }
          action={
            // Pinned to one tested release, so this never changes the version — it repairs
            // an install that has gone bad, which detection cannot see because it only
            // checks the file exists.
            <button
              type="button"
              className={alassReady ? "secondary" : undefined}
              onClick={() => void onDownloadRecommendedAlass()}
              disabled={downloadIsActive || busyAction === "downloadAlass"}
            >
              {alassReady ? "Re-download" : "Download"}
            </button>
          }
        >
          {progress("alass")}
        </DownloadRow>
      </div>
    </SettingsDisclosure>
  );
}
