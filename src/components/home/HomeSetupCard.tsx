import { DownloadProgressCard } from "../settings/DownloadProgressCard";
import type { ModelDownloadSnapshot, TranscriptionRequirement } from "../../types";

/** What each unmet requirement is called on the landing page.*/
const REQUIREMENT_LABEL: Record<string, string> = {
  whisper: "Transcription engine and model",
  ffmpeg: "Audio processing",
  vad: "Speech detector",
};

/**Failures worth reporting on Home.*/
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

      {!modelReady && modelLabel && modelDiskSize ? (
        <p className="microcopy">
          Your model is set to {modelLabel} ({modelDiskSize}).
        </p>
      ) : null}

      {failureMessage ? (
        <>
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
