import type { CalendarDay } from "../../types";
import { formatDuration, shiftDay, shortDate } from "../../lib/progressFormat";
import { ChartTooltip, useChartTooltip } from "./ChartTooltip";

const WINDOW = 30;
const MINUTE = 60000;
const STEP_MINUTES = [1, 2, 5, 10, 15, 30, 60, 120, 240, 480];
const MAX_INTERVALS = 3;
const LABEL_EVERY = 7;

type Slot = { day: string; ms: number | null };
type Counted = { day: string; ms: number };

// The smallest clean step that covers the peak in at most three intervals, so the axis
// reads 0 / 10m / 20m rather than 0 / 6m 26s / 12m 52s.
function scaleFor(peakMs: number): { top: number; ticks: number[] } {
  const peakMinutes = Math.max(peakMs / MINUTE, 1);
  const stepMinutes =
    STEP_MINUTES.find((step) => Math.ceil(peakMinutes / step) <= MAX_INTERVALS) ??
    STEP_MINUTES[STEP_MINUTES.length - 1];
  const intervals = Math.ceil(peakMinutes / stepMinutes);
  const ticks = Array.from({ length: intervals + 1 }, (_, index) => index * stepMinutes * MINUTE);
  return { top: ticks[ticks.length - 1], ticks };
}

export function DayChart({ days, today }: { days: CalendarDay[]; today: string }) {
  const { frame, tip, handlers } = useChartTooltip();

  // The last thirty days ending today. A day the report does not cover is no data, and a
  // null stays null: it gets no height at all rather than a bar of zero.
  const byDay = new Map(days.map((entry) => [entry.day, entry.combinedMs]));
  const slots: Slot[] = Array.from({ length: WINDOW }, (_, index) => {
    const day = shiftDay(today, index - (WINDOW - 1));
    return { day, ms: byDay.get(day) ?? null };
  });

  const measured = slots.filter((slot): slot is Counted => slot.ms !== null);
  const total = measured.reduce((sum, slot) => sum + slot.ms, 0);
  const peak = measured.reduce<Counted | null>(
    (best, slot) => (best === null || best.ms < slot.ms ? slot : best),
    null,
  );
  const { top, ticks } = scaleFor(peak === null ? 0 : peak.ms);
  const leading = slots.findIndex((slot) => slot.ms !== null);
  const uncounted = leading === -1 ? WINDOW : leading;

  const summary =
    measured.length === 0
      ? "Nothing has been counted in the last 30 days."
      : `${formatDuration(total)} in the last 30 days${
          peak !== null && peak.ms > 0
            ? `, most on ${shortDate(peak.day)} (${formatDuration(peak.ms)})`
            : ""
        }.`;

  return (
    <div className="day-chart">
      <div className="day-chart-head">
        <span className="day-chart-title">Time played, last 30 days</span>
        <span className="day-chart-total">
          {measured.length === 0 ? "—" : formatDuration(total)}
        </span>
      </div>

      <div className="day-chart-frame" ref={frame} {...handlers}>
        <div className="day-chart-y" aria-hidden="true">
          {ticks.map((tick) => (
            <span key={tick} style={{ bottom: `${(tick / top) * 100}%` }}>
              {formatDuration(tick)}
            </span>
          ))}
        </div>

        <div className="day-chart-plot" role="img" aria-label={summary}>
          {ticks.map((tick) => (
            <i
              key={tick}
              className="day-chart-grid"
              style={{ bottom: `${(tick / top) * 100}%` }}
            />
          ))}

          {uncounted > 0 ? (
            <div
              className="day-chart-uncounted"
              style={{ width: `${(uncounted / WINDOW) * 100}%` }}
            >
              {uncounted >= 6 ? <span>Not counted yet</span> : null}
            </div>
          ) : null}

          <div className="day-chart-columns">
            {slots.map((slot) => {
              if (slot.ms === null) {
                return <span key={slot.day} className="day-chart-slot" />;
              }
              const isPeak = peak !== null && slot.day === peak.day && slot.ms > 0;
              return (
                <span
                  key={slot.day}
                  className="day-chart-slot is-counted"
                  tabIndex={0}
                  aria-label={`${shortDate(slot.day)}: ${formatDuration(slot.ms)}`}
                  data-tip-value={formatDuration(slot.ms)}
                  data-tip-label={slot.day === today ? `${shortDate(slot.day)} · today` : shortDate(slot.day)}
                >
                  {slot.ms === 0 ? (
                    <i className="day-chart-zero" />
                  ) : (
                    <i
                      className="day-chart-bar"
                      style={{ height: `${(slot.ms / top) * 100}%` }}
                    >
                      {isPeak ? (
                        <b className="day-chart-peak">{formatDuration(slot.ms)}</b>
                      ) : null}
                    </i>
                  )}
                </span>
              );
            })}
          </div>
        </div>

        <div className="day-chart-x" aria-hidden="true">
          {slots.map((slot, index) => {
            const fromEnd = WINDOW - 1 - index;
            if (fromEnd % LABEL_EVERY !== 0) {
              return null;
            }
            // Today is pinned to the right edge; the rest sit centred under their day.
            const place =
              fromEnd === 0
                ? { right: 0 }
                : { left: `${((index + 0.5) / WINDOW) * 100}%`, transform: "translateX(-50%)" };
            return (
              <span key={slot.day} style={place}>
                {fromEnd === 0 ? "Today" : shortDate(slot.day)}
              </span>
            );
          })}
        </div>

        <ChartTooltip tip={tip} />
      </div>

      <details className="viz-table">
        <summary>Show the last 30 days as a list</summary>
        <table>
          <thead>
            <tr>
              <th scope="col">Day</th>
              <th scope="col">Time played</th>
            </tr>
          </thead>
          <tbody>
            {[...slots].reverse().map((slot) => (
              <tr key={slot.day}>
                <td>{shortDate(slot.day)}</td>
                <td>{slot.ms === null ? "not counted" : formatDuration(slot.ms)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </details>
    </div>
  );
}
