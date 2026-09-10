import type { Measured } from "../../types";

/// The only component allowed to paint a number on the Progress page.
///
/// Everything the backend measures crosses the wire as a `Measured`, which carries whether
/// it could be worked out at all. This is where that decision becomes pixels, and it is one
/// component rather than a rule at each call site because a rule at each call site is a rule
/// somebody forgets: a single `?? 0` between the backend's `null` and a bare `<span>` puts a
/// confident zero on screen for a reading that never happened.
///
/// The value prop is `T | null` on purpose and there is no code path from null to a number.
///
/// Every state renders into the same fixed-height slot, so the page does not reflow as a
/// metric changes state. That is what makes the dash read as deliberate rather than as
/// something that failed to load.
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

/// A whole metric: its label, its number, and whatever the number needs said about it.
///
/// `qualified` covers the two states that carry a real value under a caveat — a reading
/// taken before the vocabulary settings changed, and one that could not read everything.
/// Both keep their number at full size and gain a dated line, because a dated answer beats
/// no answer as long as the date is on it.
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

/// Reads a `Measured` from the backend into what `Metric` needs.
///
/// Kept beside the component so the mapping from wire status to treatment lives in one
/// place. `unavailable` is the only status that yields a null value, and it does so because
/// the backend sent null — never because a status string was unrecognised.
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

/// One small stat in the summary grid.
///
/// The same rule as `Metric`, at a smaller size: `value` is `string | null` and null draws
/// a dash rather than a number. A tile is where a zero would be easiest to slip in, because
/// a grid of them reads as a block and one wrong cell hides in it.
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
