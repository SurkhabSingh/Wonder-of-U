import type { AppBootstrap, ProgressReport } from "../../types";
import { Metric, readMeasured } from "./MeasuredValue";
import {
  formatCompared,
  formatCount,
  formatDay,
  formatDelta,
  formatPercent,
} from "../../lib/progressFormat";

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
  onGoToStudyPicks,
}: {
  bootstrap: AppBootstrap;
  report: ProgressReport | null;
  readCount: number;
  failed: boolean;
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

      <div className="progress-row">
        <article className="panel">
          <Metric
            label="Words you know"
            value={
              knownWords.status === "ready" || knownWords.wordCount > 0
                ? formatCount(knownWords.wordCount)
                : null
            }
            note={
              knownWords.status === "ready" || knownWords.wordCount > 0
                ? null
                : "Choose the decks you study, and this fills in."
            }
          />
        </article>
      </div>

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
