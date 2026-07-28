import type { ReactNode } from "react";
import type { WhisperAssetUpdateResult } from "../../types";
import { SettingsDisclosure } from "./SettingsDisclosure";

// A capability the app manages for you: what it lets you do, whether it is available right
// now, and the action that changes that.
//
// One box, the same box, in both states — that is the whole point of this component. The
// three sections built on it do the same job and had stopped looking like they did: one had
// an empty body when everything was fine, one had a check button and a result card, one had
// a re-download button and a leftover progress bar. The differences were accidents of the
// order they were written in, not decisions.
//
// So the body is exactly one status box. Green when the capability works, carrying what it
// lets you do; amber when it does not, carrying what to do about it. The right-hand end
// holds whatever action is genuinely on offer — and says so plainly when none is, rather
// than showing a button that cannot do anything.
export function CapabilitySection({
  title,
  toolName,
  description,
  ready,
  readyLabel = "Ready",
  missingLabel = "Missing",
  callToAction,
  action,
  onCheck,
  checkBusy = false,
  checkResult,
  children,
}: {
  title: string;
  /// What does the work. See `SettingsDisclosure`.
  toolName?: string;
  /// What the capability lets you do, shown while it is working. This used to be a tooltip
  /// on the heading; it reads better as the thing filling the box.
  description: string;
  ready: boolean;
  readyLabel?: string;
  /// "Missing" for something the app needs, "Optional" for something it can do without.
  missingLabel?: string;
  /// Shown instead of `description` when unavailable. Says what to do, not what is wrong.
  callToAction: string;
  /// The button in the box: Download when unavailable, or an install when a check has
  /// turned up a newer version.
  action?: ReactNode;
  /// Omit entirely when there is nothing meaningful to check — the box then says so instead
  /// of offering a button that would report nothing.
  onCheck?: () => void;
  checkBusy?: boolean;
  checkResult?: WhisperAssetUpdateResult | null;
  /// Progress, and anything else that only exists mid-operation.
  children?: ReactNode;
}) {
  return (
    <SettingsDisclosure
      title={title}
      toolName={toolName}
      // Something you still have to set up opens itself; something that works stays shut.
      defaultOpen={!ready}
      tone={ready ? "ready" : "attention"}
      status={
        <span
          className={`status-chip status-chip-${ready ? "success" : "warning"}`}
        >
          {ready ? readyLabel : missingLabel}
        </span>
      }
    >
      <div className={`update-card is-row ${ready ? "current" : "available"}`}>
        <div>
          <strong>{ready ? description : callToAction}</strong>
          {ready && checkResult ? (
            <p className="microcopy">{checkResult.message}</p>
          ) : null}
        </div>

        <div className="capability-actions">
          {action}
          {ready && onCheck ? (
            <button
              type="button"
              className="secondary"
              onClick={onCheck}
              disabled={checkBusy}
            >
              {checkBusy ? "Checking…" : "Check"}
            </button>
          ) : null}
          {ready && !onCheck ? (
            <span className="microcopy">No updates to check for.</span>
          ) : null}
        </div>
      </div>

      {children}
    </SettingsDisclosure>
  );
}
