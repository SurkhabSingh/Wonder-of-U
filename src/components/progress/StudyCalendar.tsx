import type { CalendarDay, CalendarSpan } from "../../types";
import {
  formatDuration,
  monthName,
  monthOf,
  shiftDay,
  shortDate,
  weekdayOf,
} from "../../lib/progressFormat";
import { ChartTooltip, useChartTooltip } from "./ChartTooltip";

const WEEKS = 53;
const DAYS_IN_WEEK = 7;
const MINUTE = 60000;
const WEEKDAY_LABELS = ["Mon", "", "Wed", "", "Fri", "", ""];

// Upper bounds of the time steps, in minutes. The legend prints these, so the scale is
// read off the page rather than guessed.
const STEPS = [15, 30, 60];

function step(ms: number): number {
  const minutes = ms / MINUTE;
  const below = STEPS.findIndex((limit) => minutes < limit);
  return below === -1 ? STEPS.length + 1 : below + 1;
}

type Reading = { state: string; value: string; label: string };

// The state comes from the report, never from comparing dates here. No entry means no layer
// can speak for the day; a null means time was not being counted. Neither is a zero.
function read(day: string, entry: CalendarDay | undefined, today: string): Reading {
  if (day > today) {
    return { state: "is-future", value: "", label: "" };
  }
  if (!entry || (entry.combinedMs === null && !entry.addedMaterial)) {
    return { state: "is-void", value: "No data", label: shortDate(day) };
  }
  if (entry.combinedMs === null) {
    return {
      state: "is-material",
      value: "Material added",
      label: `${shortDate(day)} · before time tracking`,
    };
  }
  const extras = [
    entry.madeCard ? "card made" : null,
    entry.addedMaterial ? "material added" : null,
  ].filter(Boolean);
  return {
    state: entry.combinedMs === 0 ? "is-zero" : `t${step(entry.combinedMs)}`,
    value: formatDuration(entry.combinedMs),
    label: [shortDate(day), ...extras].join(" · "),
  };
}

export function StudyCalendar({ span, today }: { span: CalendarSpan; today: string }) {
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
    .filter(
      (entry) =>
        (entry.combinedMs !== null && entry.combinedMs > 0) ||
        entry.madeCard ||
        entry.addedMaterial,
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
          aria-label={`A year of days, time counted since ${shortDate(span.countedFrom)}. The same days are listed below.`}
        >
          {cells.map((cell) => {
            const reading = read(cell.day, cell.entry, today);
            return (
              <i
                key={cell.day}
                className={`progress-cal-cell ${reading.state}`}
                data-tip-value={reading.value || undefined}
                data-tip-label={reading.label || undefined}
              >
                {cell.entry?.madeCard ? <b className="progress-cal-card" /> : null}
              </i>
            );
          })}
        </div>

        <ChartTooltip tip={tip} />
      </div>

      <div className="progress-cal-legend">
        <span className="progress-cal-scale">
          <span className="progress-cal-scale-name">Time played</span>
          {[
            ["is-zero", "0"],
            ["t1", "<15m"],
            ["t2", "15m"],
            ["t3", "30m"],
            ["t4", "1h+"],
          ].map(([state, label]) => (
            <span key={state} className="progress-cal-step">
              <i className={`progress-cal-cell ${state}`} />
              {label}
            </span>
          ))}
        </span>
        <span className="progress-cal-key">
          <i className="progress-cal-cell is-material" />
          Material added before time tracking
        </span>
        <span className="progress-cal-key">
          <i className="progress-cal-cell is-zero">
            <b className="progress-cal-card" />
          </i>
          Card made
        </span>
        <span className="progress-cal-key">
          <i className="progress-cal-cell is-void" />
          No data
        </span>
      </div>

      <p className="progress-cal-since">
        Time tracking since {shortDate(span.countedFrom)}
        {span.droppedEvidence > 0
          ? ` · ${span.droppedEvidence} item${
              span.droppedEvidence === 1 ? "" : "s"
            } had no usable date`
          : ""}
      </p>

      {noted.length > 0 ? (
        <details className="viz-table">
          <summary>Show every day with activity as a list</summary>
          <table>
            <thead>
              <tr>
                <th scope="col">Day</th>
                <th scope="col">Time played</th>
                <th scope="col">Card made</th>
                <th scope="col">Material added</th>
              </tr>
            </thead>
            <tbody>
              {noted.map((entry) => (
                <tr key={entry.day}>
                  <td>{shortDate(entry.day)}</td>
                  <td>
                    {entry.combinedMs === null
                      ? "not counted"
                      : formatDuration(entry.combinedMs)}
                  </td>
                  <td>{entry.madeCard ? "yes" : entry.madeCard === null ? "—" : "no"}</td>
                  <td>{entry.addedMaterial ? "yes" : "no"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </details>
      ) : null}
    </div>
  );
}
