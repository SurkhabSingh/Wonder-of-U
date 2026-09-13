import type { Measured } from "../../types";

export function MeasuredValue({
  value,
  unit,
}: {
  value: string | null;
  unit?: string;
}) {
  if (value === null) {
    return (
      <div className="metric-slot">
        <span className="metric-value is-absent" aria-label="not measured">
          &mdash;
        </span>
      </div>
    );
  }
  return (
    <div className="metric-slot">
      <span className="metric-value">{value}</span>
      {unit ? <span className="metric-unit">{unit}</span> : null}
    </div>
  );
}

/// `qualified` is the two states carrying a real value under a caveat. Both keep the
/// number at full size and gain a dated line: a dated answer beats no answer.
export function Metric({
  label,
  value,
  unit,
  qualified,
  asOf,
  note,
  trend,
}: {
  label: string;
  value: string | null;
  unit?: string;
  qualified?: boolean;
  asOf?: string | null;
  note?: string | null;
  trend?: React.ReactNode;
}) {
  return (
    <div className="progress-metric">
      <div className="metric-label">{label}</div>
      <div className="metric-slot">
        {value === null ? (
          <span className="metric-value is-absent" aria-label="not measured">
            &mdash;
          </span>
        ) : (
          <>
            <span className={qualified ? "metric-value is-qualified" : "metric-value"}>
              {value}
            </span>
            {unit ? <span className="metric-unit">{unit}</span> : null}
          </>
        )}
      </div>
      {trend ? <div className="metric-trend">{trend}</div> : null}
      {asOf ? (
        <div className="metric-asof">
          <span className="metric-dot" aria-hidden="true" />
          {asOf}
        </div>
      ) : null}
      {note ? (
        <p className={qualified ? "metric-note is-warning" : "metric-note"}>{note}</p>
      ) : null}
    </div>
  );
}

/// `unavailable` is the only status yielding a null value, and only because the backend
/// sent null — never because a status string was unrecognised.
export function readMeasured<T>(measured: Measured<T>): {
  known: boolean;
  qualified: boolean;
  value: T | null;
  reason: string | null;
  asOfMs: number | null;
} {
  return {
    known: measured.status === "known",
    qualified: measured.status === "stale" || measured.status === "partial",
    value: measured.value,
    reason: measured.reason,
    asOfMs: measured.asOfMs,
  };
}

export function StatTile({ label, value }: { label: string; value: string | null }) {
  return (
    <div className="progress-tile">
      <div className="progress-tile-label">{label}</div>
      <div className="progress-tile-value">
        {value === null ? (
          <span className="is-absent" aria-label="not measured">
            &mdash;
          </span>
        ) : (
          value
        )}
      </div>
    </div>
  );
}
