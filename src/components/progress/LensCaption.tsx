import type { CalendarSpan, Measured } from "../../types";
import { formatClock, formatCount, shortDate } from "../../lib/progressFormat";
import type { Lens } from "./lenses";

function undated(count: number, noun: string): string | null {
  if (count === 0) {
    return null;
  }
  return `${formatCount(count)} ${noun}${count === 1 ? "" : "s"} had no usable date`;
}

function joined(parts: (string | null)[]): string {
  return parts.filter((part): part is string => part !== null).join(" · ");
}

export function LensCaption({
  lens,
  span,
  today,
  minedCards,
  counting,
  onCount,
}: {
  lens: Lens;
  span: CalendarSpan;
  today: string;
  minedCards: Measured<number> | null;
  counting: boolean;
  onCount: () => void;
}) {
  if (lens === "time") {
    return (
      <>
        {span.countedFrom === null
          ? "Time could not be read, so no day shows any"
          : `Time tracking since ${shortDate(span.countedFrom)}`}
      </>
    );
  }

  if (lens === "material") {
    return (
      <>
        {joined([
          span.materialFrom === null
            ? "Nothing in your library has a date yet"
            : "Counted on the day each item entered your library",
          undated(span.droppedEvidence, "item"),
        ])}
      </>
    );
  }

  // Dated by the app's day, as the chart is, so an answer given at 2 a.m. and the empty
  // days after it name the same day.
  const counted = span.cards;
  return (
    <>
      {counted === null
        ? "Cards are counted from Anki while it is open"
        : joined([
            counted.on === today
              ? `Counted from Anki at ${formatClock(counted.atMs)}`
              : `Counted from Anki on ${shortDate(counted.on)}`,
            undated(counted.undated, "card"),
          ])}
      {minedCards !== null && minedCards.status === "known" ? null : (
        <button
          type="button"
          className="ghost progress-cal-count"
          disabled={counting}
          onClick={onCount}
        >
          {counting ? "Counting…" : "Count now"}
        </button>
      )}
    </>
  );
}
