import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { errorMessage } from "../../lib/errors";

export function TroubleshootingSettings({
  logPath,
  onError,
}: {
  logPath: string;
  onError: (message: string) => void;
}) {
  const [copied, setCopied] = useState(false);

  const openFolder = async () => {
    try {
      await invoke("open_log_folder");
    } catch (error) {
      onError(errorMessage(error, "The log folder could not be opened."));
    }
  };

  const copyDiagnostics = async () => {
    try {
      const text = await invoke<string>("copy_diagnostics");
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 2500);
    } catch (error) {
      onError(errorMessage(error, "The summary could not be copied."));
    }
  };

  return (
    <>
      <header className="panel-header">
        <div>
          <p className="panel-kicker">Settings</p>
          <h2>Troubleshooting</h2>
        </div>
      </header>

      <div className="settings-card">
        <p className="microcopy">
          If something goes wrong, these are the two things worth sending. The
          summary describes this machine and what is installed. The log records
          what the app did, and it lives in the folder below.
        </p>

        <div className="action-row inline-actions">
          <button type="button" onClick={() => void copyDiagnostics()}>
            {copied ? "Copied" : "Copy summary"}
          </button>
          <button
            type="button"
            className="secondary"
            onClick={() => void openFolder()}
          >
            Open log folder
          </button>
        </div>

        <p className="microcopy">
          The log holds file locations, recording names, and what each action
          did. Your Windows account name is replaced before anything is written,
          but recording names come from what was said, so read it before sending.
        </p>

        {logPath ? (
          <p className="path-copy" title={logPath}>
            {logPath}
          </p>
        ) : null}
      </div>
    </>
  );
}
