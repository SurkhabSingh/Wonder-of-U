import type { BackfillPoint, ReadingPoint, WordsPoint } from "../../types";
import { formatCount, formatDay, formatDelta, formatPercent } from "../../lib/progressFormat";
import { ChartTooltip, useChartTooltip } from "./ChartTooltip";
import { FloatingList } from "./FloatingList";
import { SwitchGroup } from "./SwitchGroup";

type Mark = {
  atMs: number;
  day: string;
  value: number;
  // False when the step from the mark before could not be measured: that link is dashed.
  joined: boolean;
  // False for a replayed day: the line runs through it, but only measurements get a dot.
  dot: boolean;
  tip: string;
  detail: string;
};

type Domain = { low: number; high: number };

const EDGE = 6;

function spread(marks: Mark[]): (atMs: number) => number {
  const first = marks[0].atMs;
  const last = marks[marks.length - 1].atMs;
  if (last === first) {
    return () => 50;
  }
  return (atMs) => EDGE + ((atMs - first) / (last - first)) * (100 - 2 * EDGE);
}

function LevelChart({
  title,
  headline,
  marks,
  domain,
  formatTick,
  note,
  caption,
  listLabel,
  valueHeading,
  seamAt,
  rows = marks,
}: {
  title: string;
  headline: string;
  marks: Mark[];
  domain: Domain;
  formatTick: (value: number) => string;
  note: string;
  caption: string;
  listLabel: string;
  valueHeading: string;
  seamAt?: number;
  rows?: Mark[];
}) {
  const { frame, tip, handlers } = useChartTooltip();
  const x = spread(marks);
  const y = (value: number) => ((value - domain.low) / (domain.high - domain.low)) * 100;
  const first = marks[0];
  const last = marks[marks.length - 1];

  return (
    <div className="level-chart">
      <div className="level-chart-head">
        <span className="level-chart-title">{title}</span>
        <span className="level-chart-value">{headline}</span>
      </div>

      <div className="level-chart-frame" ref={frame} {...handlers}>
        <div className="level-chart-y" aria-hidden="true">
          <span>{formatTick(domain.high)}</span>
          <span>{formatTick(domain.low)}</span>
        </div>

        <div
          className="level-chart-plot"
          role="img"
          aria-label={
            marks.length > 1
              ? `${title}: ${headline}, ${first.day} to ${last.day}`
              : `${title}: ${headline}`
          }
        >
          {/* The links stretch with the plot; the dots are HTML so they stay round. */}
          <svg viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
            {marks.slice(1).map((mark, index) => {
              const before = marks[index];
              return (
                <line
                  key={mark.atMs}
                  className={mark.joined ? "level-chart-link" : "level-chart-link is-unmeasured"}
                  x1={x(before.atMs)}
                  y1={100 - y(before.value)}
                  x2={x(mark.atMs)}
                  y2={100 - y(mark.value)}
                  vectorEffect="non-scaling-stroke"
                />
              );
            })}
          </svg>
          {seamAt === undefined ? null : (
            <i className="level-chart-seam" style={{ left: `${x(seamAt)}%` }} aria-hidden="true" />
          )}
          {marks.map((mark) => (
            <span
              key={mark.atMs}
              // A replayed day carries no dot, but it still answers the pointer.
              className={mark.dot ? "level-chart-dot" : "level-chart-dot is-plain"}
              style={{ left: `${x(mark.atMs)}%`, top: `${100 - y(mark.value)}%` }}
              tabIndex={mark.dot ? 0 : undefined}
              aria-label={mark.dot ? `${mark.day}: ${mark.tip}, ${mark.detail}` : undefined}
              data-tip-value={mark.tip}
              data-tip-label={`${mark.day} · ${mark.detail}`}
            />
          ))}
        </div>

        <div className="level-chart-x" aria-hidden="true">
          <span>{first.day}</span>
          {marks.length > 1 ? <span>{last.day}</span> : null}
        </div>

        <ChartTooltip tip={tip} />
      </div>

      <p className="level-chart-caption">{marks.length > 1 ? caption : note}</p>

      <FloatingList label={listLabel}>
        <table>
          <thead>
            <tr>
              <th scope="col">Day</th>
              <th scope="col">{valueHeading}</th>
              <th scope="col">What changed</th>
            </tr>
          </thead>
          <tbody>
            {[...rows].reverse().map((mark) => (
              <tr key={mark.atMs}>
                <td>{mark.day}</td>
                <td>{mark.tip}</td>
                <td>{mark.detail}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </FloatingList>
    </div>
  );
}

function points(value: number): string {
  return `${formatDelta(value)} point${Math.abs(value) === 1 ? "" : "s"}`;
}

function words(count: number): string {
  return `${formatCount(count)} word${count === 1 ? "" : "s"}`;
}

function transcripts(count: number): string {
  return `${formatCount(count)} transcript${count === 1 ? "" : "s"}`;
}

export function ReadingChart({ readings, now }: { readings: ReadingPoint[]; now: Date }) {
  const marks: Mark[] = readings.map((reading, index) => {
    const step =
      index === 0
        ? "first reading"
        : reading.step !== null
          ? `${formatDelta(reading.step)} on the same ${transcripts(reading.compared)}`
          : reading.notCompared === "settingsChanged"
            ? "word-list settings changed, not compared"
            : "not enough of the same text to compare";
    return {
      atMs: reading.atMs,
      day: formatDay(reading.atMs, now),
      value: reading.gained,
      joined: reading.notCompared === null,
      dot: true,
      tip: points(reading.gained),
      detail: `${step} · ${formatPercent(reading.coverage)}% of your library at the time`,
    };
  });
  const values = marks.map((mark) => mark.value);
  const low = Math.min(0, ...values);
  // At least two points tall, so a gain of a tenth is drawn as the small step it is.
  const domain = { low, high: Math.max(low + 2, ...values) };

  return (
    <LevelChart
      title="Reading growth"
      headline={points(values[values.length - 1])}
      marks={marks}
      domain={domain}
      formatTick={(value) => (value === 0 ? "0" : formatDelta(value))}
      note="The line starts once you learn more words and refresh your word list."
      caption="Counted on the transcripts both readings had, unchanged, so new material never moves it."
      listLabel="Show the line as a list"
      valueHeading="Growth"
    />
  );
}

// A line that spans years needs no point per day: one in every few draws the same, and the
// list underneath still holds every day.
const MOST_REPLAYED = 120;

function thinned<T>(points: T[]): T[] {
  if (points.length <= MOST_REPLAYED) {
    return points;
  }
  const step = Math.ceil(points.length / MOST_REPLAYED);
  return points.filter((_, index) => index % step === 0 || index === points.length - 1);
}

export type WordsRange = "month" | "quarter" | "year" | "all";

const DAY_MS = 86_400_000;

const WORDS_RANGES: Record<WordsRange, { label: string; days: number | null; covers: string }> = {
  month: { label: "30 days", days: 30, covers: "the last 30 days" },
  quarter: { label: "90 days", days: 90, covers: "the last 90 days" },
  year: { label: "Year", days: 365, covers: "the past year" },
  all: { label: "All", days: null, covers: "" },
};

const WORDS_RANGE_ORDER: WordsRange[] = ["month", "quarter", "year", "all"];

function rangeStart(range: WordsRange, now: Date): number {
  const days = WORDS_RANGES[range].days;
  return days === null ? -Infinity : now.getTime() - days * DAY_MS;
}

/** A range is offered when it keeps two points or more and leaves one out; a choice that is not
 * offered draws All. The switch and the chart both read this, so they cannot disagree. */
export function wordsRangeView(
  counts: WordsPoint[],
  backfill: BackfillPoint[],
  choice: WordsRange,
  now: Date,
): { offered: WordsRange[]; shown: WordsRange } {
  const times = [...backfill, ...counts].map((point) => point.atMs);
  const offered = WORDS_RANGE_ORDER.filter((id) => {
    const kept = times.filter((at) => at >= rangeStart(id, now)).length;
    return id === "all" || (kept >= 2 && kept < times.length);
  });
  return { offered, shown: offered.includes(choice) ? choice : "all" };
}

export function WordsRangeSwitch({
  ranges,
  range,
  onRange,
}: {
  ranges: WordsRange[];
  range: WordsRange;
  onRange: (range: WordsRange) => void;
}) {
  return (
    <SwitchGroup
      label="How far back Words you know reaches"
      options={ranges.map((id) => ({ id, label: WORDS_RANGES[id].label }))}
      value={range}
      onChange={onRange}
    />
  );
}

export function WordsChart({
  counts,
  backfill,
  now,
  choice,
}: {
  counts: WordsPoint[];
  backfill: BackfillPoint[];
  now: Date;
  choice: WordsRange;
}) {
  const range = wordsRangeView(counts, backfill, choice, now).shown;
  const replayedDay = (point: BackfillPoint): Mark => ({
    atMs: point.atMs,
    // Named by the day it counts for, not by the small hours it runs into.
    day: formatDay(new Date(`${point.day}T12:00:00`).getTime(), now),
    value: point.words,
    joined: true,
    dot: false,
    tip: words(point.words),
    detail: "replayed from your review log",
  });
  const measured: Mark[] = counts.map((count, index) => {
    const before = index > 0 ? counts[index - 1] : null;
    const change = count.settingsChanged
      ? "word-list settings changed"
      : before === null
        ? "first count"
        : `${count.words >= before.words ? "+" : "-"}${formatCount(Math.abs(count.words - before.words))} since ${formatDay(before.atMs, now)}`;
    return {
      atMs: count.atMs,
      day: formatDay(count.atMs, now),
      value: count.words,
      joined: !count.settingsChanged,
      dot: true,
      tip: words(count.words),
      detail: change,
    };
  });
  const start = rangeStart(range, now);
  const replayedShown = backfill.filter((point) => point.atMs >= start);
  const measuredShown = measured.filter((mark) => mark.atMs >= start);
  const replayed = thinned(replayedShown).map(replayedDay);
  const marks = [...replayed, ...measuredShown];
  const values = marks.map((mark) => mark.value);
  const least = Math.min(...values);
  const most = Math.max(...values);
  // At least fifty words either side, so a handful learned is not drawn as a cliff.
  const pad = Math.max((most - least) * 0.15, 50);
  const domain = {
    low: Math.max(0, Math.floor((least - pad) / 10) * 10),
    high: Math.ceil((most + pad) / 10) * 10,
  };

  return (
    <LevelChart
      title="Words you know"
      headline={formatCount(values[values.length - 1])}
      marks={marks}
      domain={domain}
      formatTick={formatCount}
      note="One count so far. The line starts with your next word-list refresh."
      caption={
        replayed.length === 0
          ? "Your word list each time it was refreshed."
          : `Replayed from Anki's review log up to ${replayed[replayed.length - 1].day}, and your word list each time it was refreshed after that.`
      }
      listLabel={
        range === "all"
          ? "Show every count as a list"
          : `Show every count from ${WORDS_RANGES[range].covers} as a list`
      }
      valueHeading="Words"
      seamAt={replayed.length > 0 && measuredShown.length > 0 ? measuredShown[0].atMs : undefined}
      rows={[...replayedShown.map(replayedDay), ...measuredShown]}
    />
  );
}
