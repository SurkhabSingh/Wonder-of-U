import type { ReactNode } from "react";

// One managed binary, as a row in the Downloads list.
//
// This was a whole collapsible section per binary — three sections spending a screen each to
// say a sentence and offer one button. As rows they are directly comparable, which is what
// they always should have been: the same kind of thing, three times.
//
// The action deliberately differs per row rather than being forced into one shared word,
// because the three are not doing the same thing. FFmpeg tracks a rolling build, so
// re-downloading genuinely updates it. yt-dlp can be asked whether it is behind. alass is
// pinned to a tested release, so a re-fetch repairs what is there and never changes the
// version. One label for all three would have to be wrong about two of them.
export function DownloadRow({
  title,
  toolName,
  description,
  version,
  ready,
  readyLabel = "Ready",
  missingLabel = "Missing",
  note,
  action,
  children,
}: {
  title: string;
  /// The binary this row manages, muted beside the title.
  toolName: string;
  description: string;
  /// What the installed copy reports for itself. Shown whenever it is known, not only after
  /// a check — it is the thing worth knowing at a glance.
  version?: string | null;
  ready: boolean;
  readyLabel?: string;
  /// "Missing" for something the app needs, "Optional" for something it can do without.
  missingLabel?: string;
  /// The outcome of the last check, when this row has one to run.
  note?: string | null;
  action: ReactNode;
  /// Progress, rendered inside this row so it is obvious which download it belongs to.
  children?: ReactNode;
}) {
  return (
    <div className="download-row">
      <div className="download-row-main">
        <div className="download-row-text">
          <p className="download-row-title">
            {title} <span className="download-row-tool">{toolName}</span>
            {version ? (
              <span className="download-row-version">{version}</span>
            ) : null}
          </p>
          <p className="microcopy">{note ?? description}</p>
        </div>

        <span
          className={`status-chip status-chip-${ready ? "success" : "warning"}`}
        >
          {ready ? readyLabel : missingLabel}
        </span>

        <div className="download-row-actions">{action}</div>
      </div>

      {children}
    </div>
  );
}
