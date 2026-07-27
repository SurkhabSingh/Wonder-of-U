import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import core from "../../lib/scannerCore";
import type { LookupEntry, LookupResult } from "../../types";

// The dictionary popup, built against the Anki add-on's own markup.
//
// Every class name here is one `popup.css` already styles — that stylesheet is vendored
// unchanged at `src/styles/lookupPopup.css`, is scoped entirely under `.anki-lookup-popup`,
// and carries no Anki-specific selectors. Matching its DOM rather than inventing our own is
// what makes the popup look like the one the user already reads during review, and it means
// restyling happens in one place for both.
//
// Positioned by hand rather than with a popover library because the anchor is a `DOMRect`
// measured from a text Range at hover time, not a mounted element — the one thing every
// popover primitive assumes it has. `core.popupPosition` does the flip-and-clamp, shared
// with the add-on.

const MARGIN = 12;
const GAP = 8;
const WIDTH = 360;
const MAX_HEIGHT = 420;

export function LookupPopup({
  anchor,
  result,
  isLoading,
  error,
  theme,
  fontFamily,
  fontSizePx,
  onClose,
}: {
  anchor: DOMRect;
  result: LookupResult | null;
  isLoading: boolean;
  error: string | null;
  /// "light" | "dark" — mirrored onto `data-theme`, which is how `popup.css` themes itself.
  theme: string;
  fontFamily: string;
  fontSizePx: number;
  onClose: () => void;
}) {
  const panelRef = useRef<HTMLElement | null>(null);
  const [placement, setPlacement] = useState({
    left: anchor.left,
    top: anchor.bottom + GAP,
  });

  // Measured after paint: the height depends on how many entries came back, and placing it
  // before that would flip the wrong way on a long result.
  useLayoutEffect(() => {
    const height = panelRef.current?.getBoundingClientRect().height ?? 0;
    const size = core.clampPopupSize(
      WIDTH,
      height || MAX_HEIGHT,
      window.innerWidth,
      window.innerHeight,
      MARGIN,
    );
    const next = core.popupPosition(
      { left: anchor.left, top: anchor.top, bottom: anchor.bottom },
      size,
      window.innerWidth,
      window.innerHeight,
      MARGIN,
      GAP,
    );
    setPlacement({ left: next.left, top: next.top });
  }, [anchor, result, isLoading, error]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  // One tab per dictionary, in the order the backend ranked them — the add-on's "source
  // rail". Grouping here rather than in the backend keeps `LookupResult` a flat list.
  const groups = useMemo(() => {
    const byDictionary = new Map<string, LookupEntry[]>();
    for (const entry of result?.entries ?? []) {
      const key = entry.dictionary || "Dictionary";
      const existing = byDictionary.get(key);
      if (existing) {
        existing.push(entry);
      } else {
        byDictionary.set(key, [entry]);
      }
    }
    return [...byDictionary.entries()];
  }, [result]);

  const [activeTab, setActiveTab] = useState(0);
  useEffect(() => {
    setActiveTab(0);
  }, [result]);

  const status = !isLoading && !error && result && result.status !== "ready";

  return (
    <section
      ref={panelRef}
      className="anki-lookup-popup anki-lookup--visible"
      data-depth="0"
      data-theme={theme}
      role="dialog"
      aria-live="polite"
      aria-label="Dictionary"
      style={{
        left: placement.left,
        top: placement.top,
        width: WIDTH,
        maxHeight: MAX_HEIGHT,
        display: "flex",
        // The add-on sets these two inline on the popup root; the whole stylesheet is
        // em-relative off the font size, so one value rescales everything.
        ["--anki-lookup-font-family" as string]: fontFamily || "inherit",
        ["--anki-lookup-font-size" as string]: `${fontSizePx}px`,
      }}
    >
      <header className="anki-lookup__header">
        <button
          type="button"
          className="anki-lookup__header-control anki-lookup__close"
          onClick={onClose}
          aria-label="Close the dictionary"
        >
          <span className="anki-lookup__close-icon" aria-hidden="true" />
        </button>
      </header>

      {groups.length > 1 ? (
        <div className="anki-lookup__tabs" role="tablist" aria-label="Lookup sources">
          <span className="anki-lookup__tabs-label">Sources</span>
          {groups.map(([dictionary], index) => (
            <button
              key={dictionary}
              type="button"
              role="tab"
              className={index === activeTab ? "anki-lookup__tab--active" : ""}
              aria-selected={index === activeTab}
              tabIndex={index === activeTab ? 0 : -1}
              onClick={() => setActiveTab(index)}
            >
              {dictionary}
            </button>
          ))}
        </div>
      ) : null}

      <div className="anki-lookup__body">
        {isLoading ? (
          <div className="anki-lookup__loading">
            <span className="anki-lookup__spinner" aria-hidden="true" />
            Looking up…
          </div>
        ) : null}
        {error ? (
          <div className="anki-lookup__status anki-lookup__status--error">{error}</div>
        ) : null}
        {status ? <div className="anki-lookup__status">{result.message}</div> : null}
        <div className="anki-lookup__panels">
          {groups.map(([dictionary, entries], index) =>
            index === activeTab ? (
              <div key={dictionary} className="anki-lookup__panel" role="tabpanel">
                {entries.map((entry, entryIndex) => (
                  <EntryView key={`${dictionary}-${entryIndex}`} entry={entry} />
                ))}
              </div>
            ) : null,
          )}
        </div>
      </div>
    </section>
  );
}

function EntryView({ entry }: { entry: LookupEntry }) {
  return (
    <article className="anki-lookup__entry">
      <div className="anki-lookup__entry-heading">
        <div className="anki-lookup__headword">
          <strong>{entry.expression}</strong>
          {entry.reading && entry.reading !== entry.expression ? (
            <span className="anki-lookup__reading">{entry.reading}</span>
          ) : null}
        </div>
      </div>

      {entry.pitchAccents.length > 0 || entry.frequencies.length > 0 ? (
        <div className="anki-lookup__lexical-metadata">
          {entry.pitchAccents.length > 0 ? (
            <div className="anki-lookup__lexical-row" aria-label="Pronunciation">
              {entry.pitchAccents.slice(0, 3).map((pitch, index) => (
                <PitchContour
                  key={index}
                  reading={entry.reading || entry.expression}
                  position={pitch.position}
                />
              ))}
            </div>
          ) : null}
          {entry.frequencies.length > 0 ? (
            <div className="anki-lookup__lexical-row" aria-label="Frequency">
              {entry.frequencies
                .filter((item) => item.displayValue)
                .slice(0, 3)
                .map((item, index) => (
                  <span
                    key={index}
                    className="anki-lookup__lexical-item anki-lookup__lexical-item--frequency"
                  >
                    <span className="anki-lookup__lexical-source">{item.dictionary}</span>
                    <span className="anki-lookup__frequency-value">
                      {item.displayValue}
                    </span>
                  </span>
                ))}
            </div>
          ) : null}
        </div>
      ) : null}

      {/* Why a conjugated form matched: 食べた came from 食べる, rather than asking the
          reader to take the match on trust. */}
      {entry.inflectionReasons.length > 0 ? (
        <div className="anki-lookup__inflection">
          <span className="anki-lookup__inflection-icon" aria-hidden="true" />
          {entry.inflectionReasons.map((reason, index) => (
            <span key={index}>
              {index > 0 ? (
                <span className="anki-lookup__inflection-separator">←</span>
              ) : null}
              <span className="anki-lookup__inflection-step">{reason}</span>
            </span>
          ))}
        </div>
      ) : null}

      {/* Rendered as text, deliberately: the add-on's own popup uses `textContent` for
          definitions, so text is the format they are actually in. */}
      <ol className="anki-lookup__definitions">
        {entry.definitions.map((definition, index) => (
          <li key={index}>{definition}</li>
        ))}
      </ol>
    </article>
  );
}

/// The pitch contour, drawn the add-on's way: one span per mora carrying `data-pitch`,
/// with the overline and the downstep bar coming from CSS borders. The mora split and the
/// high/low pattern both come from the shared core, so a small kana never gets its own
/// step here while it is merged there.
function PitchContour({
  reading,
  position,
}: {
  reading: string;
  position: number;
}) {
  const morae = core.japaneseMorae(reading);
  const levels = core.pitchLevels(morae.length, position);
  return (
    <span className="anki-lookup__lexical-item anki-lookup__lexical-item--pitch">
      <span className="anki-lookup__pitch" lang="ja">
        {morae.map((mora, index) => (
          <span
            key={index}
            className="anki-lookup__pitch-mora"
            data-pitch={levels[index] ? "high" : "low"}
            data-downstep={levels[index] && !levels[index + 1] ? "true" : undefined}
          >
            {mora}
          </span>
        ))}
      </span>
      <span className="anki-lookup__pitch-position">[{position}]</span>
    </span>
  );
}
