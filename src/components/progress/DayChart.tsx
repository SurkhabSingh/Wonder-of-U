import type { CalendarSpan } from "../../types";
import { shiftDay, shortDate } from "../../lib/progressFormat";
import { ChartTooltip, useChartTooltip } from "./ChartTooltip";
import type { LensSpec } from "./lenses";

const WINDOW = 30;
const MAX_INTERVALS = 3;
const LABEL_EVERY = 7;
const LABELLED_GAP = 6;

type Slot = { day: string; value: number | null; detail: string | null };
type Counted = { day: string; value: number };
type Gap = { start: number; end: number; label: string };

// The smallest clean step that covers the peak in at most three intervals, so the axis
// reads 0 / 10m / 20m rather than 0 / 6m 26s / 12m 52s.
function scaleFor(peak: number, steps: number[]): { top: number; ticks: number[] } {
  const reach = Math.max(peak, steps[0]);
  const size =
    steps.find((step) => Math.ceil(reach / step) <= MAX_INTERVALS) ?? steps[steps.length - 1];
  const intervals = Math.ceil(reach / size);
  const ticks = Array.from({ length: intervals + 1 }, (_, index) => index * size);
  return { top: ticks[ticks.length - 1], ticks };
}

// Every run of days with no number gets one band, so a count that stopped early reads as
// stopped rather than as a row of zeros.
function gapsIn(slots: Slot[], lens: LensSpec, span: CalendarSpan): Gap[] {
  const gaps: Gap[] = [];
  let index = 0;
  while (index < slots.length) {
    if (slots[index].value !== null) {
      index += 1;
      continue;
    }
    const start = index;
    while (index < slots.length && slots[index].value === null) {
      index += 1;
    }
    gaps.push({ start, end: index, label: lens.gap(slots[start].day, span).band });
  }
  return gaps;
}

export function DayChart({
  span,
  today,
  lens,
}: {
  span: CalendarSpan;
  today: string;
  lens: LensSpec;
}) {
  const { frame, tip, handlers } = useChartTooltip();

  const byDay = new Map(span.days.map((entry) => [entry.day, entry]));
  const slots: Slot[] = Array.from({ length: WINDOW }, (_, index) => {
    const day = shiftDay(today, index - (WINDOW - 1));
    const entry = byDay.get(day);
    return entry === undefined
      ? { day, value: null, detail: null }
      : { day, value: lens.valueOf(entry), detail: lens.detailOf(entry) };
  });

  const measured = slots.filter((slot): slot is Slot & Counted => slot.value !== null);
  const total = measured.reduce((sum, slot) => sum + slot.value, 0);
  const peak = measured.reduce<Counted | null>(
    (best, slot) => (best === null || best.value < slot.value ? slot : best),
    null,
  );
  const { top, ticks } = scaleFor(peak === null ? 0 : peak.value, lens.ticks);
  const gaps = gapsIn(slots, lens, span);

  const summary =
    measured.length === 0
      ? `${lens.title}: nothing counted in the last 30 days.`
      : `${lens.title}: ${lens.total(total)} in the last 30 days${
          peak !== null && peak.value > 0
            ? `, most on ${shortDate(peak.day)} (${lens.format(peak.value)})`
            : ""
        }.`;

  return (
    <div className="day-chart">
      <div className="day-chart-head">
        <span className="day-chart-title">{lens.title}, last 30 days</span>
        <span className="day-chart-total">
          {measured.length === 0 ? "—" : lens.total(total)}
        </span>
      </div>

      <div className="day-chart-frame" ref={frame} {...handlers}>
        <div className="day-chart-y" aria-hidden="true">
          {ticks.map((tick) => (
            <span key={tick} style={{ bottom: `${(tick / top) * 100}%` }}>
              {lens.formatTick(tick)}
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

          {gaps.map((gap) => (
            <div
              key={gap.start}
              className="day-chart-gap"
              style={{
                left: `${(gap.start / WINDOW) * 100}%`,
                width: `${((gap.end - gap.start) / WINDOW) * 100}%`,
              }}
            >
              {gap.end - gap.start >= LABELLED_GAP ? <span>{gap.label}</span> : null}
            </div>
          ))}

          <div className="day-chart-columns">
            {slots.map((slot) => {
              if (slot.value === null) {
                return <span key={slot.day} className="day-chart-slot" />;
              }
              const isPeak = peak !== null && slot.day === peak.day && slot.value > 0;
              const when = slot.day === today ? `${shortDate(slot.day)} · today` : shortDate(slot.day);
              return (
                <span
                  key={slot.day}
                  className="day-chart-slot is-counted"
                  tabIndex={0}
                  aria-label={`${shortDate(slot.day)}: ${lens.format(slot.value)}`}
                  data-tip-value={lens.format(slot.value)}
                  data-tip-label={slot.detail === null ? when : `${when} · ${slot.detail}`}
                >
                  {slot.value === 0 ? (
                    <i className="day-chart-zero" />
                  ) : (
                    <i
                      className="day-chart-bar"
                      style={{ height: `${(slot.value / top) * 100}%` }}
                    >
                      {isPeak ? (
                        <b className="day-chart-peak">{lens.format(slot.value)}</b>
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
              <th scope="col">{lens.title}</th>
              {lens.detailHeading === null ? null : <th scope="col">{lens.detailHeading}</th>}
            </tr>
          </thead>
          <tbody>
            {[...slots].reverse().map((slot) => (
              <tr key={slot.day}>
                <td>{shortDate(slot.day)}</td>
                <td>
                  {slot.value === null
                    ? lens.gap(slot.day, span).tip
                    : lens.format(slot.value)}
                </td>
                {lens.detailHeading === null ? null : <td>{slot.detail ?? ""}</td>}
              </tr>
            ))}
          </tbody>
        </table>
      </details>
    </div>
  );
}
