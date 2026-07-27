// Types for the vendored `scannerCore.js`.
//
// Deliberately a SUBSET: the add-on's module exports 33 functions, most of which serve its
// nested-popup/pin/resize/translation features that the app does not have. Declaring only
// what is used keeps this from becoming a second, drifting copy of an API we don't call.

export type Segment = {
  /// The word under the offset, as it appears in the text.
  text: string;
  /// Where that word starts, in UTF-16 code units.
  start: number;
  end: number;
};

export type ScannerCore = {
  /// The word containing `offset`, found with `Intl.Segmenter` and a Unicode fallback.
  /// Used for the immediate highlight and for identity dedupe, before the backend answers
  /// with the term it actually matched.
  segmentAt(text: string, offset: number, locale?: string): Segment | null;

  /// `max(0, interval - (now - previousStart))` — the throttle floor. Zero when it has
  /// already been at least `interval` since the last lookup started, so the common case
  /// fires immediately rather than always waiting out a debounce.
  lookupDelay(now: number, previousStart: number, interval: number): number;

  /// Splits a kana reading into morae, merging small kana (ゃゅょっ) onto the preceding
  /// character — the reason a naive per-character split draws the wrong pitch contour.
  japaneseMorae(reading: string): string[];

  /// One boolean per mora plus one for the following particle: true is high.
  /// Accepts either an integer downstep position or an explicit "LHHH" string.
  pitchLevels(moraCount: number, position: number | string): boolean[];

  /// Sentence-with-offset around a scanned word, sanitised so an Anki cloze cannot be
  /// corrupted by unbalanced braces.
  sentenceContextAt(
    text: string,
    offset: number,
    locale?: string,
  ): { text: string; offset: number; term: string } | null;

  /// Flip-and-clamp placement against the viewport.
  popupPosition(
    anchor: { left: number; top: number; bottom: number } | null,
    size: { width: number; height: number },
    viewportWidth: number,
    viewportHeight: number,
    margin: number,
    gap: number,
    userSized?: boolean,
  ): { left: number; top: number; maxHeight?: number };

  clampPopupSize(
    width: number,
    height: number,
    viewportWidth: number,
    viewportHeight: number,
    margin: number,
  ): { width: number; height: number };
};

declare const core: ScannerCore;
export default core;
