import type { ReactNode } from "react";
import { SettingsDisclosure } from "./SettingsDisclosure";

// A capability the app manages for you: what it lets you do, whether it is available
// right now, and the action that changes that.
//
// The shape this replaces spent the most space on the state that carries the least
// information. When everything was fine it drew a full-width card to say "ready" — which
// the status chip beside the heading had already said — and under it the file name of the
// thing that was ready. Three of those stacked is most of a screen spent confirming that
// nothing needs doing.
//
// So the card is inverted: it appears only when the capability is NOT available, where it
// earns its space by telling you what to do about it. The section is collapsed in that
// same spirit — a capability that works needs no more than its heading and its chip.
export function CapabilitySection({
  title,
  help,
  ready,
  readyLabel = "Ready",
  missingLabel = "Missing",
  callToAction,
  children,
}: {
  title: string;
  /// What the capability does and how to use it. Never how it is implemented.
  help: string;
  ready: boolean;
  readyLabel?: string;
  /// "Missing" for something the app needs, "Optional" for something it can do without.
  missingLabel?: string;
  /// Shown only when unavailable. Should say what to do, not what is wrong.
  callToAction: string;
  /// The action row, and anything with progress or a result of its own.
  children?: ReactNode;
}) {
  return (
    <SettingsDisclosure
      title={title}
      help={help}
      // Something you still have to set up opens itself; something that works stays shut.
      defaultOpen={!ready}
      status={
        <span
          className={`status-chip status-chip-${ready ? "success" : "warning"}`}
        >
          {ready ? readyLabel : missingLabel}
        </span>
      }
    >
      {ready ? null : (
        <div className="update-card available">
          <strong>{callToAction}</strong>
        </div>
      )}

      {children}
    </SettingsDisclosure>
  );
}
