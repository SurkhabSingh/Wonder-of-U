import { useEffect, useRef } from "react";

export type ConfirmOptions = {
  title?: string;
  message: string;
  okLabel?: string;
  cancelLabel?: string;
  // Styles the confirm button as destructive (red) for delete/erase actions.
  danger?: boolean;
};

/**
 * An in-app confirmation dialog styled to match the app (and its dark theme) — a
 * replacement for the OS-native `window.confirm`/`ask` popups. Escape or a backdrop
 * click cancels; the confirm button is auto-focused so Enter confirms.
 */
export function ConfirmDialog({
  options,
  onConfirm,
  onCancel,
}: {
  options: ConfirmOptions;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const confirmRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    confirmRef.current?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        onCancel();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  return (
    <div className="confirm-overlay" role="presentation" onClick={onCancel}>
      <div
        className="confirm-panel"
        role="alertdialog"
        aria-modal="true"
        onClick={(event) => event.stopPropagation()}
      >
        {options.title ? (
          <strong className="confirm-title">{options.title}</strong>
        ) : null}
        <p className="confirm-message">{options.message}</p>
        <div className="confirm-actions">
          <button type="button" className="ghost" onClick={onCancel}>
            {options.cancelLabel ?? "Cancel"}
          </button>
          <button
            ref={confirmRef}
            type="button"
            className={options.danger ? "confirm-danger" : ""}
            onClick={onConfirm}
          >
            {options.okLabel ?? "OK"}
          </button>
        </div>
      </div>
    </div>
  );
}
