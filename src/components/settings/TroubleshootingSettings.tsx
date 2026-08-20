import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { errorMessage } from "../../lib/errors";

/**
 * Everything a user needs to report a problem.
 *
 * Two things, because they answer different halves. The diagnostics block is what a bug report
 * form asks for and what a person will actually paste; the log file is the optional attachment
 * that says what happened, and it needs a way to be found at all — a filesystem path in a chat
 * message is not one.
 */
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
      // A rejected invoke carries the backend's plain string, never an Error, so an
      // `instanceof` test discards every reason the command can give.
      onError(errorMessage(error, "The log folder could not be opened."));
    }
  };

  const copyDiagnostics = async () => {
    try {
      const text = await invoke<string>("copy_diagnostics");
      await navigator.clipboard.writeText(text);
      setCopied(true);
      // Long enough to read, short enough that the button does not look stuck.
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

        {/* Named plainly, because the file is about to be handed to someone else and nobody
            should have to open it to find out what is in it. */}
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
