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

/// How much of the library the user can read, and whether that is going up.
///
/// The page answers one question — am I getting better — so comprehension leads and
/// everything else is context. It never contacts Anki: a closed Anki must not be able to
/// turn a measurement that was taken into a number that is missing.
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

  // Never looked yet, as opposed to looked and found nothing. Until a read has succeeded
  // there is nothing honest to draw, and a page of dashes would claim we had checked.
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
        // A condition the app is in rather than one action's outcome, so it is stated on
        // the page it concerns and not raised as a toast. Nothing is lost — the file is
        // never written over when it cannot be read.
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
                  // Neither coloured nor signed. A zero shown in the colour of growth is a
                  // number dressed as something it is not, and "+0.0" reads as a gain of
                  // nothing rather than as no gain.
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
              // One reading is a number, not a trend. Saying so beats drawing a flat line
              // that implies no progress was made.
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
          // The caveats live here rather than beside the numbers they qualify. Each is a
          // fact about how the totals were reached, and a tile is too small to carry one
          // without shouting — but leaving them off would make every total read as whole
          // when some of them are not.
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
              minedCards?.reason ?? null,
            ]
              .filter(Boolean)
              .join(" · ")}
          </p>
        ) : null}
      </article>

      {coverage && coverage.value === null ? (
        // Carbon's rule: where more than one metric can be unavailable at once, the page
        // carries one action rather than each panel carrying its own.
        <button type="button" className="ghost" onClick={onGoToStudyPicks}>
          Go to Study Picks
        </button>
      ) : null}
    </section>
  );
}
