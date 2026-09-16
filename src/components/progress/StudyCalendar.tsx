import type { CalendarDay, CalendarSpan } from "../../types";
import { formatDuration } from "../../lib/progressFormat";

const WEEKS = 53;
const DAYS_IN_WEEK = 7;
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const WEEKDAY_LABELS = ["Mon", "", "Wed", "", "Fri", "", ""];

function level(ms: number): number {
  const minutes = ms / 60000;
  if (minutes < 1) return 0;
  if (minutes < 15) return 1;
  if (minutes < 30) return 2;
  if (minutes < 60) return 3;
  return 4;
}

function shift(day: string, byDays: number): string {
  const at = new Date(`${day}T00:00:00Z`);
  at.setUTCDate(at.getUTCDate() + byDays);
  return at.toISOString().slice(0, 10);
}

function weekdayOf(day: string): number {
  return (new Date(`${day}T00:00:00Z`).getUTCDay() + 6) % 7;
}

function monthOf(day: string): number {
  return Number(day.slice(5, 7)) - 1;
}

function shortDate(day: string): string {
  return `${MONTHS[monthOf(day)]} ${Number(day.slice(8, 10))}`;
}

// The region comes from the report, not from comparing dates here: no entry means no layer
// can speak for the day, and a null means time was not being counted. Neither is a zero.
function classOf(day: string, entry: CalendarDay | undefined, today: string): string {
  if (day > today) return "progress-cal-cell is-future";
  if (!entry) return "progress-cal-cell is-void";
  if (entry.combinedMs === null) {
    return entry.addedMaterial
      ? "progress-cal-cell is-material"
      : "progress-cal-cell is-uncounted";
  }
  return `progress-cal-cell l${level(entry.combinedMs)}`;
}

function titleOf(day: string, entry: CalendarDay | undefined, today: string): string {
  if (day > today) return "";
  if (!entry) return `${shortDate(day)} — before your library`;
  if (entry.combinedMs === null) {
    return `${shortDate(day)} — time not counted yet${
      entry.addedMaterial ? " · material added" : ""
    }`;
  }
  const said = [
    `${shortDate(day)} — ${
      entry.combinedMs === 0 ? "nothing played" : formatDuration(entry.combinedMs)
    }`,
  ];
  if (entry.madeCard) said.push("a card was made");
  if (entry.addedMaterial) said.push("material added");
  return said.join(" · ");
}

export function StudyCalendar({ span, today }: { span: CalendarSpan; today: string }) {
  if (span.firstDay === null) {
    return null;
  }

  const byDay = new Map(span.days.map((entry) => [entry.day, entry]));
  const gridStart = shift(today, -weekdayOf(today) - (WEEKS - 1) * DAYS_IN_WEEK);

  const cells: { day: string; entry: CalendarDay | undefined }[] = [];
  for (let row = 0; row < DAYS_IN_WEEK; row += 1) {
    for (let week = 0; week < WEEKS; week += 1) {
      const day = shift(gridStart, week * DAYS_IN_WEEK + row);
      cells.push({ day, entry: byDay.get(day) });
    }
  }

  // A month is labelled on the first column it owns, skipped when the next is too close to fit.
  const months: { week: number; label: string }[] = [];
  for (let week = 0; week < WEEKS; week += 1) {
    const monday = shift(gridStart, week * DAYS_IN_WEEK);
    const previous = week === 0 ? null : shift(monday, -DAYS_IN_WEEK);
    if (previous === null || monthOf(previous) !== monthOf(monday)) {
      months.push({ week, label: MONTHS[monthOf(monday)] });
    }
  }
  const shownMonths = months.filter(
    (month, index) => index === months.length - 1 || months[index + 1].week - month.week >= 3,
  );

  const counted = span.days.filter(
    (entry) => entry.combinedMs !== null && entry.combinedMs > 0,
  ).length;
  const columns = { gridTemplateColumns: `repeat(${WEEKS}, minmax(0, 1fr))` };

  return (
    <div className="progress-cal">
      <div className="progress-cal-body">
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
          aria-label={`${counted} days with time counted since ${shortDate(span.countedFrom)}`}
        >
          {cells.map((cell) => (
            <i
              key={cell.day}
              className={classOf(cell.day, cell.entry, today)}
              title={titleOf(cell.day, cell.entry, today)}
            >
              {cell.entry?.madeCard ? <b className="progress-cal-card" /> : null}
            </i>
          ))}
        </div>
      </div>

      <div className="progress-cal-legend">
        <span className="progress-cal-ramp">
          Less
          <i className="progress-cal-cell l0" />
          <i className="progress-cal-cell l1" />
          <i className="progress-cal-cell l2" />
          <i className="progress-cal-cell l3" />
          <i className="progress-cal-cell l4" />
          More
        </span>
        <span className="progress-cal-key">
          <i className="progress-cal-cell l2">
            <b className="progress-cal-card" />
          </i>
          Card made
        </span>
        <span className="progress-cal-key">
          <i className="progress-cal-cell is-material" />
          Material added, time not counted
        </span>
        <span className="progress-cal-key">
          <i className="progress-cal-cell is-uncounted" />
          Not counted yet
        </span>
      </div>

      <p className="progress-cal-since">
        Counting time since {shortDate(span.countedFrom)}
        {span.droppedEvidence > 0
          ? ` · ${span.droppedEvidence} item${
              span.droppedEvidence === 1 ? "" : "s"
            } had no usable date`
          : ""}
      </p>
    </div>
  );
}
