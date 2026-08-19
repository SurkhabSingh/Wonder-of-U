import { DownloadProgressCard } from "../settings/DownloadProgressCard";
import type { ModelDownloadSnapshot, TranscriptionRequirement } from "../../types";

/**
 * What each unmet requirement is called on the landing page.
 *
 * Labels only — this does NOT decide what gets downloaded. That answer lives in Rust beside
 * the readiness check that produced these ids, so the two cannot disagree about what is
 * missing. An id with no label here is skipped rather than shown raw.
 */
const REQUIREMENT_LABEL: Record<string, string> = {
  whisper: "Transcription engine and model",
  ffmpeg: "Audio processing",
  vad: "Speech detector",
};

/**
 * Failures worth reporting on Home.
 *
 * The download snapshot is global, so a dictionary or yt-dlp download started from Settings
 * fails into the same field. Those belong to their own Settings card, not under a heading
 * about transcription.
 */
const TRANSCRIPTION_KINDS = ["model", "runtime", "ffmpeg"];

export function HomeSetupCard({
  setupIncomplete,
  requirements,
  modelReady,
  modelLabel,
  modelDiskSize,
  isDownloadingAssets,
  downloadIsActive,
  downloadSnapshot,
  downloadBusy,
  onDownloadMissing,
  onTogglePause,
  onCancelDownload,
}: {
  setupIncomplete: boolean;
  requirements: TranscriptionRequirement[];
  modelReady: boolean;
  modelLabel: string | null;
  modelDiskSize: string | null;
  isDownloadingAssets: boolean;
  downloadIsActive: boolean;
  downloadSnapshot: ModelDownloadSnapshot;
  downloadBusy: boolean;
  onDownloadMissing: () => void;
  onTogglePause: () => void;
  onCancelDownload: () => void;
}) {
  if (!setupIncomplete && !isDownloadingAssets) {
    return null;
  }

  // The phase, not the download status. The status goes terminal between queue items while the
  // phase stays held, so gating on the status would flip this card back to its offer state —
  // with a live Download button — in the middle of a run the user already started.
  if (isDownloadingAssets) {
    return (
      <article className="panel home-setup-card">
        {/* Neutral on purpose: a dictionary or subtitle-sync download started from Settings
            lands here too, and calling that "setting up transcription" would be false. */}
        <p className="panel-kicker">Downloads</p>
        <h2>Download in progress</h2>
        <DownloadProgressCard
          snapshot={downloadSnapshot}
          downloadIsActive={downloadIsActive}
          onTogglePause={onTogglePause}
          onCancel={onCancelDownload}
        />
        <p className="microcopy">
          A recording cannot start until this finishes.
          {downloadSnapshot.queuedRemaining > 0
            ? " Cancelling stops the ones still waiting too."
            : ""}
        </p>
      </article>
    );
  }

  const missing = requirements
    .filter((requirement) => !requirement.ready)
    .map((requirement) => REQUIREMENT_LABEL[requirement.id])
    .filter((label): label is string => label !== undefined);

  // A failed download leaves nothing else on Home once its toast expires, and the queue drops
  // everything still waiting when one item fails — so without this a run can end with the
  // engine installed, audio processing never attempted, and no surface saying so.
  const failureMessage =
    downloadSnapshot.status === "failed" &&
    downloadSnapshot.kind !== null &&
    TRANSCRIPTION_KINDS.includes(downloadSnapshot.kind)
      ? downloadSnapshot.message
      : null;

  return (
    <article className="panel home-setup-card">
      <p className="panel-kicker">Setup</p>
      <h2>Set up transcription</h2>
      <p className="microcopy">
        Recording works already. Turning a recording into text needs{" "}
        {/* The list below is not always plural: cancelling a model download between its two
            files leaves only the speech detector missing. */}
        {missing.length === 1 ? "one more thing" : "a few more things"} on this
        computer.
      </p>

      {missing.length > 0 ? (
        <ul className="home-setup-missing">
          {missing.map((label) => (
            <li key={label}>{label}</li>
          ))}
        </ul>
      ) : null}

      {/* The size is disclosed before the press, not after it. The model is by far the largest
          of these, and it is whichever one the settings name — which on an existing install can
          be the 2.9 GiB option. */}
      {!modelReady && modelLabel && modelDiskSize ? (
        <p className="microcopy">
          Your model is set to {modelLabel} ({modelDiskSize}).
        </p>
      ) : null}

      {failureMessage ? (
        <>
          {/* Two paragraphs, not one sentence joined to another. The snapshot's message is
              whatever failed said, and it does not reliably end in a full stop — a download
              error ending in a URL ran straight into the line below it. */}
          <p className="microcopy home-setup-failure">{failureMessage}</p>
          <p className="microcopy">Anything still waiting was not started.</p>
        </>
      ) : null}

      <div className="action-row">
        <button type="button" onClick={onDownloadMissing} disabled={downloadBusy}>
          Download what's missing
        </button>
      </div>
    </article>
  );
}
