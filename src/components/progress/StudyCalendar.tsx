import type { ReactNode } from "react";
import type { CalendarDay, CalendarSpan } from "../../types";
import { monthName, monthOf, shiftDay, shortDate, weekdayOf } from "../../lib/progressFormat";
import { ChartTooltip, useChartTooltip } from "./ChartTooltip";
import { stepOf, type LensSpec } from "./lenses";

const WEEKS = 53;
const DAYS_IN_WEEK = 7;
const WEEKDAY_LABELS = ["Mon", "", "Wed", "", "Fri", "", ""];
const SCALE = ["is-zero", "t1", "t2", "t3", "t4"];

type Reading = { state: string; value: string; label: string };

// The state comes from the report, never from comparing dates here. No entry means no layer
// can speak for the day, and a null means this one cannot. Neither is a zero.
function read(
  day: string,
  entry: CalendarDay | undefined,
  today: string,
  lens: LensSpec,
  span: CalendarSpan,
): Reading {
  if (day > today) {
    return { state: "is-future", value: "", label: "" };
  }
  if (!entry) {
    return { state: "is-void", value: "No data", label: shortDate(day) };
  }
  const value = lens.valueOf(entry);
  if (value === null) {
    return { state: "is-void", value: lens.gap(day, span).tip, label: shortDate(day) };
  }
  const detail = lens.detailOf(entry);
  return {
    state: value === 0 ? "is-zero" : `t${stepOf(value, lens.steps)}`,
    value: lens.format(value),
    label: detail === null ? shortDate(day) : `${shortDate(day)} · ${detail}`,
  };
}

export function StudyCalendar({
  span,
  today,
  lens,
  caption,
}: {
  span: CalendarSpan;
  today: string;
  lens: LensSpec;
  caption: ReactNode;
}) {
  const { frame, tip, handlers } = useChartTooltip();
  if (span.firstDay === null) {
    return null;
  }

  const byDay = new Map(span.days.map((entry) => [entry.day, entry]));
  const gridStart = shiftDay(today, -weekdayOf(today) - (WEEKS - 1) * DAYS_IN_WEEK);

  const cells: { day: string; entry: CalendarDay | undefined }[] = [];
  for (let row = 0; row < DAYS_IN_WEEK; row += 1) {
    for (let week = 0; week < WEEKS; week += 1) {
      const day = shiftDay(gridStart, week * DAYS_IN_WEEK + row);
      cells.push({ day, entry: byDay.get(day) });
    }
  }

  // A month is labelled on the first column it owns, skipped when the next is too close to fit.
  const months: { week: number; label: string }[] = [];
  for (let week = 0; week < WEEKS; week += 1) {
    const monday = shiftDay(gridStart, week * DAYS_IN_WEEK);
    if (week === 0 || monthOf(shiftDay(monday, -DAYS_IN_WEEK)) !== monthOf(monday)) {
      months.push({ week, label: monthName(monthOf(monday)) });
    }
  }
  const shownMonths = months.filter(
    (month, index) => index === months.length - 1 || months[index + 1].week - month.week >= 3,
  );

  const noted = [...span.days]
    .reverse()
    .map((entry) => ({ entry, value: lens.valueOf(entry) }))
    .filter((row): row is { entry: CalendarDay; value: number } =>
      row.value !== null && row.value > 0,
    );
  const columns = { gridTemplateColumns: `repeat(${WEEKS}, minmax(0, 1fr))` };

  return (
    <div className="progress-cal">
      <div className="progress-cal-body" ref={frame} {...handlers}>
        <div className="progress-cal-months" style={columns} aria-hidden="true">
          {shownMonths.map((month) => (
            <span key={month.week} style={{ gridColumn: `${month.week + 1} / span 3` }}>
              {month.label}
            </span>
          ))}
        </div>

        <div className="progress-cal-weekdays" aria-hidden="true">
          {WEEKDAY_LABELS.map((label, row) => (
            <span key={row}>{label}</span>
          ))}
        </div>

        <div
          className="progress-cal-grid"
          style={columns}
          role="img"
          aria-label={`A year of days, coloured by ${lens.title.toLowerCase()}.${
            noted.length > 0 ? " The days with any are listed below." : ""
          }`}
        >
          {cells.map((cell) => {
            const reading = read(cell.day, cell.entry, today, lens, span);
            return (
              <i
                key={cell.day}
                className={`progress-cal-cell ${reading.state}`}
                data-tip-value={reading.value || undefined}
                data-tip-label={reading.label || undefined}
              />
            );
          })}
        </div>

        <ChartTooltip tip={tip} />
      </div>

      <div className="progress-cal-legend">
        <span className="progress-cal-scale">
          <span className="progress-cal-scale-name">{lens.title}</span>
          {SCALE.map((state, index) => (
            <span key={state} className="progress-cal-step">
              <i className={`progress-cal-cell ${state}`} />
              {index === 0 ? "0" : lens.stepLabels[index - 1]}
            </span>
          ))}
        </span>
        <span className="progress-cal-key">
          <i className="progress-cal-cell is-void" />
          No data
        </span>
      </div>

      <p className="progress-cal-since">{caption}</p>

      {noted.length > 0 ? (
        <details className="viz-table">
          <summary>Show every day with {lens.title.toLowerCase()} as a list</summary>
          <table>
            <thead>
              <tr>
                <th scope="col">Day</th>
                <th scope="col">{lens.title}</th>
                {lens.detailHeading === null ? null : (
                  <th scope="col">{lens.detailHeading}</th>
                )}
              </tr>
            </thead>
            <tbody>
              {noted.map(({ entry, value }) => (
                <tr key={entry.day}>
                  <td>{shortDate(entry.day)}</td>
                  <td>{lens.format(value)}</td>
                  {lens.detailHeading === null ? null : <td>{lens.detailOf(entry)}</td>}
                </tr>
              ))}
            </tbody>
          </table>
        </details>
      ) : null}
    </div>
  );
}
