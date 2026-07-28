import { whisperStatusLabel } from "../../lib/helpers";
import type { AppBootstrap, AppSettings } from "../../types";
import { SettingsDisclosure } from "./SettingsDisclosure";

function whisperStatusTone(status: string): "success" | "warning" | "error" {
  if (status === "ready") {
    return "success";
  }
  if (status === "invalid") {
    return "error";
  }
  return "warning";
}

export function WhisperStatusSettingsPage({
  activeRuntimeVersion,
  bootstrap,
  manualRuntimeOverride,
  settingsDraft,
}: {
  activeRuntimeVersion: string;
  bootstrap: AppBootstrap;
  manualRuntimeOverride: boolean;
  settingsDraft: AppSettings;
}) {
  const runtimeDisplayLabel = manualRuntimeOverride
    ? "Manual override"
    : activeRuntimeVersion;

  return (
    <SettingsDisclosure
      title="Whisper"
      defaultOpen={bootstrap.whisperDetection.status !== "ready"}
      status={
        <span
          className={`status-chip status-chip-${whisperStatusTone(
            bootstrap.whisperDetection.status,
          )}`}
        >
          {whisperStatusLabel(bootstrap.whisperDetection.status)}
        </span>
      }
    >

      <div className="meta-list compact-meta-list">
        <div title={bootstrap.whisperDetection.executablePath || "Not installed"}>
          <span className="hint-label">Runtime</span>
          <strong>
            {bootstrap.whisperDetection.cliReady
              ? `Ready (${runtimeDisplayLabel})`
              : "Missing"}
          </strong>
        </div>
        <div title={bootstrap.whisperDetection.modelPath || "Not installed"}>
          <span className="hint-label">Model</span>
          <strong>{bootstrap.whisperDetection.modelReady ? "Ready" : "Missing"}</strong>
        </div>
        <div>
          <span className="hint-label">Language</span>
          <strong>{settingsDraft.whisper.language}</strong>
        </div>
      </div>
    </SettingsDisclosure>
  );
}
