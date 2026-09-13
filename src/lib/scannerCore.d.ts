// Types for the vendored `scannerCore.js`.

export type Segment = {
  text: string;
  start: number;
  end: number;
};

export type ScannerCore = {
  segmentAt(text: string, offset: number, locale?: string): Segment | null;

  lookupDelay(now: number, previousStart: number, interval: number): number;

  japaneseMorae(reading: string): string[];

  pitchLevels(moraCount: number, position: number | string): boolean[];

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
