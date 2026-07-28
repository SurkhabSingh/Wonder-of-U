import { useState, type ReactNode } from "react";
import { TooltipBadge } from "../ui/Tooltip";

// One settings section, collapsed to its heading until it is wanted.
//
// Eleven sections were stacked on one scrolling page, every one of them expanded, so the
// page was long in proportion to how much was configurable rather than to how much needed
// attention. Collapsed, the same page is an index: every heading and every status is
// visible at once, which is strictly more than fitted on a screen before.
//
// A section opens itself when it needs something — see `defaultOpen` at each call site. On
// a fresh install that turns the page into a to-do list; once everything is working it
// closes down to a list of headings.
//
// Built on `<details>` rather than a div and a chevron: it is the element for this, and it
// brings keyboard operation, the disclosure role, and find-in-page opening a collapsed
// section for free.
export function SettingsDisclosure({
  title,
  help,
  status,
  tone = "neutral",
  defaultOpen,
  children,
}: {
  title: string;
  /// What the section is for. Reference material — read once, then never again, which is
  /// why it is behind a badge while status stays on the summary.
  help?: string;
  /// The chip on the right. Must stay readable while collapsed: the point of collapsing is
  /// that you can still see whether anything is wrong.
  status?: ReactNode;
  /// Colours the rail down the card's edge, so "is anything wrong" is answerable from the
  /// shape of the page instead of by reading every chip.
  tone?: "neutral" | "ready" | "attention" | "error";
  defaultOpen: boolean;
  children: ReactNode;
}) {
  // Deliberately not derived from `defaultOpen` after mount. A download finishing would
  // otherwise collapse the section out from under whoever was reading it.
  const [open, setOpen] = useState(defaultOpen);

  return (
    <details
      className="settings-disclosure"
      data-tone={tone}
      open={open}
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary className="settings-disclosure-summary">
        <span className="settings-disclosure-marker" aria-hidden="true" />
        <h2>{title}</h2>
        {/* Both of these swallow their own clicks. A summary toggles on any click inside
            it, so reading the hint or the status would otherwise fold the section away. */}
        {help ? (
          <span
            className="settings-disclosure-aside"
            onClick={(event) => event.preventDefault()}
          >
            <TooltipBadge label="?" description={help} />
          </span>
        ) : null}
        {status ? (
          <span
            className="settings-disclosure-status"
            onClick={(event) => event.preventDefault()}
          >
            {status}
          </span>
        ) : null}
      </summary>

      <div className="settings-disclosure-body">{children}</div>
    </details>
  );
}
