import type { AppBootstrap, Measured, ProgressReport } from "../../types";
import { Metric, StatTile, readMeasured } from "./MeasuredValue";
import {
  formatCompared,
  formatCount,
  formatDay,
  formatDelta,
  formatDuration,
  formatPercent,
} from "../../lib/progressFormat";
import { ActivityCalendar } from "./ActivityCalendar";

/// One question — am I getting better — so comprehension leads and the rest is context.
/// Never contacts Anki: a closed Anki must not hide a measurement that was taken.
export function ProgressPage({
  bootstrap,
  report,
  readCount,
  failed,
  minedCards,
  onGoToStudyPicks,
}: {
  bootstrap: AppBootstrap;
  report: ProgressReport | null;
  readCount: number;
  failed: boolean;
  minedCards: Measured<number> | null;
  onGoToStudyPicks: () => void;
}) {
  const now = new Date();
  const coverage = report ? readMeasured(report.coveragePercent) : null;
  const comparison = report?.comparison ?? null;
  const knownWords = bootstrap.knownWords;

  // Never looked, as opposed to looked and found nothing. A page of dashes would claim
  // we had checked.
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
        // A condition the app is in, not one action's outcome, so it is stated here
        // rather than raised as a toast.
        <p className="microcopy field-warning">
          Your progress history could not be opened, so there is nothing to show. Nothing
          has been written over it.
        </p>
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
                  // Neither coloured nor signed: "+0.0" reads as a gain of nothing
                  // rather than as no gain.
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
      </article>

      <article className="panel progress-summary">
        <div className="progress-tiles">
          <StatTile
            label="Words you know"
            value={
              knownWords.status === "ready" || knownWords.wordCount > 0
                ? formatCount(knownWords.wordCount)
                : null
            }
          />
          <StatTile
            label="Current streak"
            value={report ? `${formatCount(report.activity.streak.current)}d` : null}
          />
          <StatTile
            label="Longest streak"
            value={report ? `${formatCount(report.activity.streak.best)}d` : null}
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

        {report ? (
          <ActivityCalendar
            days={report.activity.days}
            today={report.activity.today}
          />
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
              report.activity.itemsWithoutADay > 0
                ? `${formatCount(report.activity.itemsWithoutADay)} on no known day`
                : null,
              // Both are readings not folded into the numbers above, and both are kept
              // in the file — so each says so, or it reports a loss that did not happen.
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
