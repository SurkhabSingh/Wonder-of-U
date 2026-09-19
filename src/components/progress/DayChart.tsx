import type { KeyboardEvent } from "react";
import type { CalendarDay, CalendarSpan } from "../../types";
import { shiftDay, shortDate } from "../../lib/progressFormat";
import { ChartTooltip, useChartTooltip } from "./ChartTooltip";
import type { LensSpec } from "./lenses";
import { DAYS_IN_WEEK, monthMarks, weeksEnding } from "./weeks";

export type Range = "month" | "quarter" | "year";

type RangeSpec = { label: string; covers: string } & (
  | { weekly: false; days: number; labelEvery: number }
  | { weekly: true; weeks: number }
);

const RANGES: Record<Range, RangeSpec> = {
  month: { label: "30 days", covers: "last 30 days", weekly: false, days: 30, labelEvery: 7 },
  quarter: { label: "90 days", covers: "last 90 days", weekly: false, days: 90, labelEvery: 14 },
  // The calendar's 53 weeks: a bar per day would be too thin to read over a year.
  year: { label: "Year", covers: "past year", weekly: true, weeks: 53 },
};

const RANGE_ORDER: Range[] = ["month", "quarter", "year"];
const MAX_INTERVALS = 3;
// A band is labelled when it spans a fifth of the plot: the six days it was sized for at 30.
const LABELLED_SHARE = 0.2;
// A peak label centred on a bar this near the axis would run into the axis labels.
const PEAK_EDGE_SHARE = 0.05;

// `day` is the Monday when a slot is a week.
type Slot = { day: string; value: number | null; detail: string | null };
type Counted = { day: string; value: number };
type Gap = { start: number; end: number; label: string };
type Mark = { index: number; text: string };

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

// Every run of slots with no number gets one band, so a count that stopped early reads as
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

function sumOf<T>(
  entries: CalendarDay[],
  pick: (entry: CalendarDay) => T | null,
  add: (sum: T, next: T) => T,
): T | null {
  return entries.reduce<T | null>((sum, entry) => {
    const value = pick(entry);
    if (value === null) {
      return sum;
    }
    return sum === null ? value : add(sum, value);
  }, null);
}

function summed(day: string, entries: CalendarDay[]): CalendarDay {
  return {
    day,
    combinedMs: sumOf(entries, (entry) => entry.combinedMs, (sum, next) => sum + next),
    cards: sumOf(
      entries,
      (entry) => entry.cards,
      (sum, next) => ({
        word: sum.word + next.word,
        line: sum.line + next.line,
        transcript: sum.transcript + next.transcript,
        unsorted: sum.unsorted + next.unsorted,
      }),
    ),
    material: sumOf(
      entries,
      (entry) => entry.material,
      (sum, next) => ({
        recordings: sum.recordings + next.recordings,
        videos: sum.videos + next.videos,
      }),
    ),
  };
}

function daySlots(
  byDay: Map<string, CalendarDay>,
  today: string,
  lens: LensSpec,
  days: number,
): Slot[] {
  return Array.from({ length: days }, (_, index) => {
    const day = shiftDay(today, index - (days - 1));
    const entry = byDay.get(day);
    return entry === undefined
      ? { day, value: null, detail: null }
      : { day, value: lens.valueOf(entry), detail: lens.detailOf(entry) };
  });
}

// A week adds up the days this view has a number for; a week with none of them has no
// number, and one with only some says how many.
function weekSlots(
  byDay: Map<string, CalendarDay>,
  today: string,
  lens: LensSpec,
  weeks: number,
): Slot[] {
  return weeksEnding(today, weeks).map((monday) => {
    const days = Array.from({ length: DAYS_IN_WEEK }, (_, offset) =>
      shiftDay(monday, offset),
    ).filter((day) => day <= today);
    const counted = days
      .map((day) => byDay.get(day))
      .filter((entry): entry is CalendarDay => entry !== undefined && lens.valueOf(entry) !== null);
    if (counted.length === 0) {
      return { day: monday, value: null, detail: null };
    }
    const week = summed(monday, counted);
    const parts = [
      lens.detailOf(week),
      counted.length < days.length ? `${counted.length} of ${days.length} days counted` : null,
    ].filter((part): part is string => part !== null);
    return {
      day: monday,
      value: lens.valueOf(week),
      detail: parts.length > 0 ? parts.join(" · ") : null,
    };
  });
}

function dayMarks(slots: Slot[], every: number): Mark[] {
  return slots.flatMap((slot, index) => {
    const fromEnd = slots.length - 1 - index;
    if (fromEnd % every !== 0) {
      return [];
    }
    return [{ index, text: fromEnd === 0 ? "Today" : shortDate(slot.day) }];
  });
}

// One stop in the tab order for the whole chart; the arrow keys move between its bars.
function stepThrough(event: KeyboardEvent<HTMLDivElement>) {
  const step = event.key === "ArrowLeft" ? -1 : event.key === "ArrowRight" ? 1 : 0;
  if (step === 0) {
    return;
  }
  const bars = [
    ...event.currentTarget.querySelectorAll<HTMLElement>(".day-chart-slot.is-counted"),
  ];
  const next = bars[bars.findIndex((bar) => bar === event.target) + step];
  if (next) {
    event.preventDefault();
    next.focus();
  }
}

function weekMarks(slots: Slot[]): Mark[] {
  return monthMarks(slots.map((slot) => slot.day)).map((month) => ({
    index: month.week,
    text: month.label,
  }));
}

export function RangeSwitch({
  range,
  onRange,
}: {
  range: Range;
  onRange: (range: Range) => void;
}) {
  return (
    <div className="progress-lens" role="group" aria-label="How far back the bars reach">
      {RANGE_ORDER.map((id) => (
        <button
          key={id}
          type="button"
          className={`progress-lens-button ${range === id ? "is-active" : ""}`}
          aria-pressed={range === id}
          onClick={() => onRange(id)}
        >
          {RANGES[id].label}
        </button>
      ))}
    </div>
  );
}

export function DayChart({
  span,
  today,
  lens,
  range: rangeId,
}: {
  span: CalendarSpan;
  today: string;
  lens: LensSpec;
  range: Range;
}) {
  const { frame, tip, handlers } = useChartTooltip();
  const range = RANGES[rangeId];

  const byDay = new Map(span.days.map((entry) => [entry.day, entry]));
  const slots = range.weekly
    ? weekSlots(byDay, today, lens, range.weeks)
    : daySlots(byDay, today, lens, range.days);
  const marks = range.weekly ? weekMarks(slots) : dayMarks(slots, range.labelEvery);
  const last = slots.length - 1;
  const newestCounted = slots.map((slot) => slot.value !== null).lastIndexOf(true);

  const measured = slots.filter((slot): slot is Slot & Counted => slot.value !== null);
  const total = measured.reduce((sum, slot) => sum + slot.value, 0);
  const peak = measured.reduce<Counted | null>(
    (best, slot) => (best === null || best.value < slot.value ? slot : best),
    null,
  );
  const { top, ticks } = scaleFor(peak === null ? 0 : peak.value, lens.ticks);
  const gaps = gapsIn(slots, lens, span);

  const detailHeading = range.weekly ? "Details" : lens.detailHeading;
  const named = (day: string) => (range.weekly ? `the week of ${shortDate(day)}` : shortDate(day));
  const heading = (slot: Slot, index: number) => {
    if (range.weekly) {
      return index === last ? "This week" : `Week of ${shortDate(slot.day)}`;
    }
    return slot.day === today ? `${shortDate(slot.day)} · today` : shortDate(slot.day);
  };

  const summary =
    measured.length === 0
      ? `${lens.title}: nothing counted in the ${range.covers}.`
      : `${lens.title}: ${lens.total(total)} in the ${range.covers}${
          peak !== null && peak.value > 0
            ? `, most ${range.weekly ? "in" : "on"} ${named(peak.day)} (${lens.format(peak.value)})`
            : ""
        }.`;

  return (
    <div className="day-chart">
      <div className="day-chart-head">
        <span className="day-chart-title">
          {lens.title}, {range.covers}
        </span>
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
                left: `${(gap.start / slots.length) * 100}%`,
                width: `${((gap.end - gap.start) / slots.length) * 100}%`,
              }}
            >
              {(gap.end - gap.start) / slots.length >= LABELLED_SHARE ? (
                <span>{gap.label}</span>
              ) : null}
            </div>
          ))}

          <div className="day-chart-columns" onKeyDown={stepThrough}>
            {slots.map((slot, index) => {
              if (slot.value === null) {
                return <span key={slot.day} className="day-chart-slot" />;
              }
              const isPeak = peak !== null && slot.day === peak.day && slot.value > 0;
              const when = heading(slot, index);
              const value = lens.format(slot.value);
              const label = slot.detail === null ? when : `${when} · ${slot.detail}`;
              return (
                <span
                  key={slot.day}
                  className="day-chart-slot is-counted"
                  tabIndex={index === newestCounted ? 0 : -1}
                  aria-label={
                    slot.detail === null
                      ? `${when}: ${value}`
                      : `${when}: ${value} · ${slot.detail}`
                  }
                  data-tip-value={value}
                  data-tip-label={label}
                >
                  {slot.value === 0 ? (
                    <i className="day-chart-zero" />
                  ) : (
                    <i
                      className="day-chart-bar"
                      style={{ height: `${(slot.value / top) * 100}%` }}
                    >
                      {isPeak ? (
                        <b
                          className={
                            (index + 0.5) / slots.length < PEAK_EDGE_SHARE
                              ? "day-chart-peak is-start"
                              : "day-chart-peak"
                          }
                        >
                          {value}
                        </b>
                      ) : null}
                    </i>
                  )}
                </span>
              );
            })}
          </div>
        </div>

        <div className="day-chart-x" aria-hidden="true">
          {marks.map((mark) => {
            // The newest slot is pinned to the right edge; the rest sit centred under theirs.
            const place =
              mark.index === last
                ? { right: 0 }
                : {
                    left: `${((mark.index + 0.5) / slots.length) * 100}%`,
                    transform: "translateX(-50%)",
                  };
            return (
              <span key={mark.index} style={place}>
                {mark.text}
              </span>
            );
          })}
        </div>

        <ChartTooltip tip={tip} />
      </div>

      <details className="viz-table">
        <summary>Show the {range.covers} as a list</summary>
        <table>
          <thead>
            <tr>
              <th scope="col">{range.weekly ? "Week of" : "Day"}</th>
              <th scope="col">{lens.title}</th>
              {detailHeading === null ? null : <th scope="col">{detailHeading}</th>}
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
                {detailHeading === null ? null : <td>{slot.detail ?? ""}</td>}
              </tr>
            ))}
          </tbody>
        </table>
      </details>
    </div>
  );
}
