import { whisperStatusLabel } from "../../lib/helpers";
import type { AppBootstrap, AppPage, AppSettings } from "../../types";
import { TooltipBadge } from "../ui/Tooltip";

export function WhisperStatusSettingsPage({
  activePage,
  activeRuntimeVersion,
  bootstrap,
  manualRuntimeOverride,
  settingsDraft,
}: {
  activePage: AppPage;
  activeRuntimeVersion: string;
  bootstrap: AppBootstrap;
  manualRuntimeOverride: boolean;
  settingsDraft: AppSettings;
}) {
  const runtimeDisplayLabel = manualRuntimeOverride
    ? "Manual override"
    : activeRuntimeVersion;

  return (
    <article className="panel settings-card" hidden={activePage !== "whisper"}>
      <header className="panel-header">
        <div>
          <p className="panel-kicker">Whisper Setup</p>
          <h2>Whisper</h2>
        </div>
        <TooltipBadge
          label={whisperStatusLabel(bootstrap.whisperDetection.status)}
          description={bootstrap.whisperDetection.message}
        />
      </header>

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
    </article>
  );
}
