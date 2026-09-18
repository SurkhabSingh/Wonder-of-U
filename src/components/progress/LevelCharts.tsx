import type { ReadingPoint, WordsPoint } from "../../types";
import { formatCount, formatDay, formatDelta, formatPercent } from "../../lib/progressFormat";
import { ChartTooltip, useChartTooltip } from "./ChartTooltip";

type Mark = {
  atMs: number;
  day: string;
  value: number;
  // False when the step from the mark before could not be measured: that link is dashed.
  joined: boolean;
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

        <div className="level-chart-plot" role="img" aria-label={`${title}: ${headline}`}>
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
          {marks.map((mark) => (
            <span
              key={mark.atMs}
              className="level-chart-dot"
              style={{ left: `${x(mark.atMs)}%`, top: `${100 - y(mark.value)}%` }}
              tabIndex={0}
              aria-label={`${mark.day}: ${mark.tip}, ${mark.detail}`}
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

      <details className="viz-table">
        <summary>{listLabel}</summary>
        <table>
          <thead>
            <tr>
              <th scope="col">Day</th>
              <th scope="col">{valueHeading}</th>
              <th scope="col">What changed</th>
            </tr>
          </thead>
          <tbody>
            {[...marks].reverse().map((mark) => (
              <tr key={mark.atMs}>
                <td>{mark.day}</td>
                <td>{mark.tip}</td>
                <td>{mark.detail}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </details>
    </div>
  );
}

function points(value: number): string {
  return `${formatDelta(value)} point${Math.abs(value) === 1 ? "" : "s"}`;
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

export function WordsChart({ counts, now }: { counts: WordsPoint[]; now: Date }) {
  const marks: Mark[] = counts.map((count, index) => {
    const before = index > 0 ? counts[index - 1] : null;
    const change =
      before === null
        ? "first count"
        : count.settingsChanged
          ? "word-list settings changed"
          : `${count.words >= before.words ? "+" : "-"}${formatCount(Math.abs(count.words - before.words))} since ${formatDay(before.atMs, now)}`;
    return {
      atMs: count.atMs,
      day: formatDay(count.atMs, now),
      value: count.words,
      joined: !count.settingsChanged,
      tip: `${formatCount(count.words)} words`,
      detail: change,
    };
  });
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
      caption="Your word list each time it was refreshed."
      listLabel="Show every count as a list"
      valueHeading="Words"
    />
  );
}
