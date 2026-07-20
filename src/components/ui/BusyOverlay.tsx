export function BusyOverlay({
  label,
  statusText,
  progress,
}: {
  label: string;
  statusText: string;
  // 0–100 while a transcription streams its progress; null/undefined for busy states
  // that have no measurable progress (the bar is hidden then).
  progress?: number | null;
}) {
  return (
    <section className="busy-overlay" role="status" aria-live="polite">
      <div className="busy-panel">
        <div className="busy-spinner" aria-hidden="true" />
        <div>
          <p className="panel-kicker">Working</p>
          <strong>{label}</strong>
          {typeof progress === "number" ? (
            <div className="progress-track" aria-hidden="true">
              <div
                className="progress-fill"
                style={{ width: `${Math.max(0, Math.min(100, progress))}%` }}
              />
            </div>
          ) : null}
          <p className="microcopy">{statusText}</p>
        </div>
      </div>
    </section>
  );
}
