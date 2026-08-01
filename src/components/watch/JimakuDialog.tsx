import { useEffect } from "react";
import { JimakuSearchPanel } from "./JimakuSearchPanel";

/**
 * A shell around the existing Jimaku panel so searching happens over the library rather than
 * pushing the list down the page.
 *
 * Hand-rolled on the same shape as `ConfirmDialog` — overlay, centred panel, Escape to close,
 * click-outside to close — because this app has no Radix Dialog and one popup does not earn a
 * new dependency. The panel inside is untouched: it already takes the video path and reports
 * the downloaded file.
 */
export function JimakuDialog({
  videoPath,
  hasApiKey,
  onDownloaded,
  onClose,
  onOpenSettings,
}: {
  videoPath: string;
  hasApiKey: boolean;
  onDownloaded: (subtitlePath: string) => void;
  onClose: () => void;
  onOpenSettings: () => void;
}) {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return (
    <div className="confirm-overlay" role="presentation" onClick={onClose}>
      <div
        className="confirm-panel modal-panel-wide"
        role="dialog"
        aria-modal="true"
        aria-label="Search Jimaku for subtitles"
        onClick={(event) => event.stopPropagation()}
      >
        <p className="confirm-title">Search Jimaku</p>

        <JimakuSearchPanel
          hasApiKey={hasApiKey}
          videoPath={videoPath}
          // Closing on success is the whole point of the dialog: the result is now visible
          // on the row behind it, so staying open would only hide what just changed.
          onDownloaded={(path) => {
            onDownloaded(path);
            onClose();
          }}
          onOpenSettings={() => {
            onClose();
            onOpenSettings();
          }}
        />

        <div className="confirm-actions">
          <button type="button" className="ghost" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
