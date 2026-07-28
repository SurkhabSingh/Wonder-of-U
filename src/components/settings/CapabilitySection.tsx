import type { ReactNode } from "react";
import { TooltipBadge } from "../ui/Tooltip";

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
// earns its space by telling you what to do about it. The explanation of what the
// capability is for moves into a badge beside the heading, because it is reference
// material — read once, then never again — while status has to stay visible.
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
  /// The action row, and anything with its own progress or result of its own.
  children?: ReactNode;
}) {
  return (
    <>
      <header className="panel-header">
        <div className="section-heading">
          <h2>{title}</h2>
          <TooltipBadge label="?" description={help} />
        </div>
        <span
          className={`status-chip status-chip-${ready ? "success" : "warning"}`}
        >
          {ready ? readyLabel : missingLabel}
        </span>
      </header>

      {ready ? null : (
        <div className="update-card available">
          <strong>{callToAction}</strong>
        </div>
      )}

      {children}
    </>
  );
}
