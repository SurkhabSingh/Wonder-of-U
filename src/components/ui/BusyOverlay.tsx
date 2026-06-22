export function BusyOverlay({
  label,
  statusText,
}: {
  label: string;
  statusText: string;
}) {
  return (
    <section className="busy-overlay" role="status" aria-live="polite">
      <div className="busy-panel">
        <div className="busy-spinner" aria-hidden="true" />
        <div>
          <p className="panel-kicker">Working</p>
          <strong>{label}</strong>
          <p className="microcopy">{statusText}</p>
        </div>
      </div>
    </section>
  );
}
