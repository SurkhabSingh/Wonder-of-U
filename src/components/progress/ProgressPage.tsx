import type { AppBootstrap, Measured, ProgressReport } from "../../types";
import { Metric, StatTile, readMeasured } from "./MeasuredValue";
import {
  formatCompared,
  formatCount,
  formatDay,
  formatDelta,
  formatDuration,
  formatMoment,
  formatPercent,
} from "../../lib/progressFormat";
import { DayChart, type Range } from "./DayChart";
import { LensCaption } from "./LensCaption";
import { ReadingChart, WordsChart } from "./LevelCharts";
import { LENSES, LENS_ORDER, type Lens } from "./lenses";
import { StudyCalendar } from "./StudyCalendar";

/// One question — am I getting better — so comprehension leads and the rest is context.
/// Never contacts Anki: a closed Anki must not hide a measurement that was taken.
export function ProgressPage({
  bootstrap,
  report,
  readCount,
  failed,
  minedCards,
  countingCards,
  onCountCards,
  refreshingWordList,
  onRefreshWordList,
  onGoToStudyPicks,
  lens,
  onLens,
  range,
  onRange,
}: {
  bootstrap: AppBootstrap;
  report: ProgressReport | null;
  readCount: number;
  failed: boolean;
  minedCards: Measured<number> | null;
  countingCards: boolean;
  onCountCards: () => void;
  refreshingWordList: boolean;
  onRefreshWordList: () => void;
  onGoToStudyPicks: () => void;
  lens: Lens;
  onLens: (lens: Lens) => void;
  range: Range;
  onRange: (range: Range) => void;
}) {
  const now = new Date();
  const coverage = report ? readMeasured(report.coveragePercent) : null;
  const immersion = report ? readMeasured(report.immersion).value : null;
  const comparison = report?.comparison ?? null;
  const knownWords = bootstrap.knownWords;

  if (readCount === 0 && !failed) {
    return (
      <section className="progress-scroll">
        <div className="progress-head">
          <h2>Progress</h2>
        </div>
        <p className="microcopy">Reading your history…</p>
      </section>
    );
  }

  const storeUnavailable = failed || (report !== null && !report.storeReadable);

  return (
    <section className="progress-scroll">
      <div className="progress-head">
        <h2>Progress</h2>
        {report && report.readings > 0 ? (
          <span className="progress-sub">
            {report.readings} reading{report.readings === 1 ? "" : "s"}
          </span>
        ) : null}
      </div>

      {storeUnavailable ? (
        <p className="microcopy field-warning">
          Your progress history could not be opened, so there is nothing to show. Nothing
          has been written over it.
        </p>
      ) : null}

      {report?.writeFailure ? (
        <div className="progress-notice" role="status">
          <strong>Progress hasn't saved since {formatMoment(report.writeFailure.sinceMs, now)}</strong>
          <span>
            Time you play is held until it can be written, as long as the app stays open.
            Cards made and readings taken in the meantime may not be kept.
          </span>
          <span className="progress-notice-detail">{report.writeFailure.reason}</span>
        </div>
      ) : null}

      <article className="panel progress-headline">
        <Metric
          label="How much you can read"
          value={
            coverage && coverage.value !== null ? formatPercent(coverage.value) : null
          }
          unit={coverage && coverage.value !== null ? "%" : undefined}
          qualified={coverage?.qualified}
          asOf={
            coverage && coverage.qualified && coverage.asOfMs !== null
              ? `measured ${formatDay(coverage.asOfMs, now)}`
              : null
          }
          note={coverage?.reason ?? null}
          trend={
            comparison ? (
              <>
                {comparison.deltaPoints === 0 ? (
                  <span>No change</span>
                ) : (
                  <span className={comparison.deltaPoints > 0 ? "up" : "down"}>
                    {formatDelta(comparison.deltaPoints)}
                  </span>
                )}{" "}
                since {formatDay(comparison.earlierTakenAtMs, now)} ·{" "}
                {formatCompared(
                  comparison.itemsCompared,
                  comparison.itemsAdded,
                  comparison.itemsChanged,
                )}
              </>
            ) : coverage?.value !== null && coverage?.value !== undefined ? (
              <>Refresh your word list again to see whether this is moving.</>
            ) : null
          }
        />
        <div className="progress-refresh">
          <button
            type="button"
            className="secondary"
            disabled={refreshingWordList}
            onClick={onRefreshWordList}
          >
            {refreshingWordList ? "Reading your collection…" : "Refresh word list"}
          </button>
        </div>
        {report && (report.levels.reading.length > 0 || report.levels.words.length > 0) ? (
          <div className="progress-levels">
            {report.levels.reading.length > 0 ? (
              <ReadingChart readings={report.levels.reading} now={now} />
            ) : null}
            {report.levels.words.length > 0 ? (
              <WordsChart counts={report.levels.words} now={now} />
            ) : null}
          </div>
        ) : null}
      </article>

      <article className="panel progress-summary">
        <div className="progress-tiles">
          <StatTile
            label="This week"
            value={immersion ? formatDuration(immersion.weekMs) : null}
          />
          <StatTile
            label="Today"
            value={immersion ? formatDuration(immersion.todayMs) : null}
          />
          <StatTile
            label="Current streak"
            value={immersion ? `${formatCount(immersion.streak.current)}d` : null}
          />
          <StatTile
            label="Longest streak"
            value={immersion ? `${formatCount(immersion.streak.longest)}d` : null}
          />
          <StatTile
            label="Words you know"
            value={
              knownWords.status === "ready" || knownWords.wordCount > 0
                ? formatCount(knownWords.wordCount)
                : null
            }
          />
          <StatTile
            label="Cards from this app"
            value={
              minedCards && minedCards.value !== null
                ? formatCount(minedCards.value)
                : null
            }
          />
          <StatTile
            label="Material"
            value={report ? formatDuration(report.library.totalMs) : null}
          />
          <StatTile
            label="Items"
            value={report ? formatCount(report.library.items) : null}
          />
          <StatTile
            label="Transcribed"
            value={report ? formatCount(report.library.transcribed) : null}
          />
          <StatTile
            label="Translated"
            value={report ? formatCount(report.library.translated) : null}
          />
        </div>

        {immersion ? (
          <p className="progress-footnote">
            Only what played in this app is counted
            {immersion.streak.todayCounted ? ", and today is on the board" : ""}.
          </p>
        ) : null}

        {report ? (
          <div className={`progress-charts lens-${lens}`}>
            <div className="progress-lens" role="group" aria-label="What the charts show">
              {LENS_ORDER.map((id) => (
                <button
                  key={id}
                  type="button"
                  className={`progress-lens-button ${lens === id ? "is-active" : ""}`}
                  aria-pressed={lens === id}
                  onClick={() => onLens(id)}
                >
                  {LENSES[id].label}
                </button>
              ))}
            </div>
            <DayChart
              span={report.calendar}
              today={report.today}
              lens={LENSES[lens]}
              range={range}
              onRange={onRange}
            />
            <StudyCalendar
              span={report.calendar}
              today={report.today}
              lens={LENSES[lens]}
              caption={
                <LensCaption
                  lens={lens}
                  span={report.calendar}
                  today={report.today}
                  minedCards={minedCards}
                  counting={countingCards}
                  onCount={onCountCards}
                />
              }
            />
          </div>
        ) : null}

        {report ? (
          <p className="progress-footnote">
            {[
              `${formatCount(report.library.japanese)} Japanese`,
              `${formatCount(report.library.recorded)} recorded`,
              `${formatCount(report.library.importedFromALink)} from a link`,
              report.library.importedFromAFile > 0
                ? `${formatCount(report.library.importedFromAFile)} from a file`
                : null,
              report.library.unknownOrigin > 0
                ? `${formatCount(report.library.unknownOrigin)} added before this was tracked`
                : null,
              report.library.itemsWithoutLength > 0
                ? `${formatCount(report.library.itemsWithoutLength)} of no known length`
                : null,
              immersion && immersion.todayUnmeasuredMs > 0
                ? `about ${formatDuration(
                    immersion.todayUnmeasuredMs,
                  )} today could not be measured`
                : null,
              report.damagedRows > 0
                ? `${formatCount(report.damagedRows)} earlier reading${
                    report.damagedRows === 1 ? "" : "s"
                  } could not be read, and ${
                    report.damagedRows === 1 ? "is" : "are"
                  } kept as ${report.damagedRows === 1 ? "it is" : "they are"}`
                : null,
              report.newerRows > 0
                ? `${formatCount(report.newerRows)} reading${
                    report.newerRows === 1 ? "" : "s"
                  } came from a newer version of the app, and ${
                    report.newerRows === 1 ? "is" : "are"
                  } kept as ${report.newerRows === 1 ? "it is" : "they are"}`
                : null,
              minedCards?.reason ?? null,
            ]
              .filter(Boolean)
              .join(" · ")}
          </p>
        ) : null}
      </article>

      {coverage && coverage.value === null ? (
        <button type="button" className="ghost" onClick={onGoToStudyPicks}>
          Go to Study Picks
        </button>
      ) : null}
    </section>
  );
}
