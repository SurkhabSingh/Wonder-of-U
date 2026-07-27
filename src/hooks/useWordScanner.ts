import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { errorMessage } from "../lib/errors";
import core from "../lib/scannerCore";
import { characterOffsetFromPoint, codePointOffset } from "../lib/scan";
import type { LookupResult } from "../types";

// Hold a modifier, hover a word, get its dictionary entry.
//
// The gesture is Yomitan's and the mechanics are the add-on's, ported rather than
// reinvented: one document-level `pointermove` in the capture phase, a modifier tracked on
// keydown/keyup AND re-checked against the live event, and a two-stage throttle. Click was
// the first attempt and was wrong — the subtitle list is something you read, and a popup on
// every click fights that.
//
// The same hook drives both surfaces. Over the video the overlay window carries
// `WS_EX_NOACTIVATE` so it never takes focus, which means it never receives key events
// either — so there the modifier state is pushed in from Rust, which polls it with
// `GetAsyncKeyState`. That is what `heldOverride` is for.

/// Marks text as scannable. The hook finds its target by walking up from the pointer, so
/// nothing has to be prop-drilled to every row.
export const SCANNABLE_CLASS = "scannable-text";
/// Names the line a scan belongs to, so a stale popup cannot highlight a line that has
/// since scrolled away or been replaced.
export const SCANNABLE_OWNER_ATTRIBUTE = "data-scan-owner";

const HIGHLIGHT_NAME = "wonder-of-u-scan";

export type ScanTarget = {
  ownerKey: string;
  /// The whole line, which is what the backend deinflects against.
  text: string;
  /// The clicked character, in UTF-16 code units — the DOM's own units.
  offset: number;
  /// Where the popup points, in viewport coordinates.
  anchor: DOMRect;
};

type Options = {
  /// "shift" | "ctrl" | "alt" | "none".
  modifier: string;
  /// "remainOpen" | "close" — what releasing the modifier does.
  releaseBehavior: string;
  debounceMs: number;
  /// Supplied by the overlay, which cannot see key events. Undefined = track keys here.
  heldOverride?: boolean;
  /// Off entirely (e.g. the overlay while mpv has no window).
  enabled?: boolean;
};

function modifierMatchesEvent(modifier: string, event: PointerEvent): boolean {
  switch (modifier) {
    case "none":
      return true;
    case "ctrl":
      return event.ctrlKey;
    case "alt":
      return event.altKey;
    default:
      return event.shiftKey;
  }
}

function modifierKeyName(modifier: string): string {
  switch (modifier) {
    case "ctrl":
      return "Control";
    case "alt":
      return "Alt";
    default:
      return "Shift";
  }
}

/// Paints the matched range without touching the DOM.
///
/// The Custom Highlight API matters here rather than being a nicety: the alternative is
/// wrapping the match in a `<span>`, and the subtitle line is re-rendered on the player's
/// clock — a wrapper would fight that, and would also invalidate the very Range used to
/// measure the popup's anchor. Guarded because it is a young API; without it the scanner
/// still works, just without the underline.
function paintHighlight(range: Range | null) {
  const highlights = (
    CSS as typeof CSS & { highlights?: Map<string, unknown> }
  ).highlights;
  const HighlightConstructor = (
    window as unknown as { Highlight?: new (...ranges: Range[]) => unknown }
  ).Highlight;
  if (!highlights || !HighlightConstructor) {
    return;
  }
  if (!range) {
    highlights.delete(HIGHLIGHT_NAME);
    return;
  }
  highlights.set(HIGHLIGHT_NAME, new HighlightConstructor(range));
}

/// A Range over `[start, end)` of the element's text, for the highlight and the anchor.
function rangeWithin(
  container: HTMLElement,
  start: number,
  end: number,
): Range | null {
  const walker = document.createTreeWalker(container, NodeFilter.SHOW_TEXT);
  let consumed = 0;
  let startNode: Text | null = null;
  let startOffset = 0;
  let endNode: Text | null = null;
  let endOffset = 0;
  let node = walker.nextNode() as Text | null;
  while (node) {
    const length = node.data.length;
    if (!startNode && consumed + length > start) {
      startNode = node;
      startOffset = start - consumed;
    }
    if (!endNode && consumed + length >= end) {
      endNode = node;
      endOffset = end - consumed;
    }
    consumed += length;
    node = walker.nextNode() as Text | null;
  }
  if (!startNode || !endNode) {
    return null;
  }
  const range = document.createRange();
  try {
    range.setStart(startNode, startOffset);
    range.setEnd(endNode, endOffset);
  } catch {
    return null;
  }
  return range;
}

export function useWordScanner({
  modifier,
  releaseBehavior,
  debounceMs,
  heldOverride,
  enabled = true,
}: Options) {
  const [target, setTarget] = useState<ScanTarget | null>(null);
  const [result, setResult] = useState<LookupResult | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const requestRef = useRef(0);
  const mountedRef = useRef(true);
  const heldRef = useRef(false);
  const pointerRef = useRef<{ x: number; y: number } | null>(null);
  const frameRef = useRef(false);
  const timerRef = useRef<number | null>(null);
  const lastStartedAtRef = useRef(0);
  // The word the popup is already showing. Moving within it must do nothing at all —
  // without this the pointer crossing one glyph would re-query the dictionary.
  const lastKeyRef = useRef<string | null>(null);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const close = useCallback(() => {
    requestRef.current += 1;
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    lastKeyRef.current = null;
    paintHighlight(null);
    setTarget(null);
    setResult(null);
    setError(null);
    setIsLoading(false);
  }, []);

  const lookUp = useCallback(async (next: ScanTarget) => {
    const token = ++requestRef.current;
    setTarget(next);
    setResult(null);
    setError(null);
    setIsLoading(true);
    try {
      const looked = await invoke<LookupResult>("lookup_term", {
        text: next.text,
        // JS counts UTF-16 code units, Rust counts code points. They agree on ordinary
        // Japanese and disagree on 𠮟る / 𩸽 / emoji.
        offset: codePointOffset(next.text, next.offset),
      });
      if (!mountedRef.current || requestRef.current !== token) {
        return;
      }
      setResult(looked);
    } catch (caught) {
      if (mountedRef.current && requestRef.current === token) {
        setError(errorMessage(caught, "That word could not be looked up."));
      }
    } finally {
      if (mountedRef.current && requestRef.current === token) {
        setIsLoading(false);
      }
    }
  }, []);

  // Widens the highlight to the term the backend actually matched — usually longer than
  // the word under the pointer, which is the clearest signal that 単品 won over 単.
  useEffect(() => {
    if (!target || result?.status !== "ready" || !result.term) {
      return;
    }
    const container = document.querySelector<HTMLElement>(
      `.${SCANNABLE_CLASS}[${SCANNABLE_OWNER_ATTRIBUTE}="${CSS.escape(target.ownerKey)}"]`,
    );
    if (!container) {
      return;
    }
    paintHighlight(
      rangeWithin(container, target.offset, target.offset + result.term.length),
    );
  }, [target, result]);

  useEffect(() => {
    if (!enabled) {
      return;
    }

    function isHeld(): boolean {
      if (modifier === "none") {
        return true;
      }
      return heldOverride ?? heldRef.current;
    }

    function processPointer() {
      frameRef.current = false;
      const pointer = pointerRef.current;
      if (!pointer || !isHeld()) {
        return;
      }
      const element = document
        .elementFromPoint(pointer.x, pointer.y)
        ?.closest<HTMLElement>(`.${SCANNABLE_CLASS}`);
      if (!element) {
        return;
      }
      const text = element.textContent ?? "";
      const offset = characterOffsetFromPoint(element, pointer.x, pointer.y);
      if (offset === null || !text) {
        return;
      }
      const word = core.segmentAt(text, offset);
      const ownerKey = element.getAttribute(SCANNABLE_OWNER_ATTRIBUTE) ?? "";
      const key = `${ownerKey} ${word ? word.start : offset} ${word ? word.text : text[offset]}`;
      if (key === lastKeyRef.current) {
        return;
      }
      lastKeyRef.current = key;

      // Painted before the debounce, deliberately: the underline follows the pointer at
      // frame rate while the query is rate-limited, which is what makes it feel instant.
      const start = word ? word.start : offset;
      const end = word ? word.end : offset + 1;
      const range = rangeWithin(element, start, end);
      paintHighlight(range);

      const anchor = range?.getBoundingClientRect() ?? element.getBoundingClientRect();
      const scanTarget: ScanTarget = { ownerKey, text, offset: start, anchor };

      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
      }
      // Zero when it has already been at least `debounceMs` since the last query started,
      // so the common case fires immediately rather than always paying a delay.
      const delay = core.lookupDelay(
        performance.now(),
        lastStartedAtRef.current,
        debounceMs,
      );
      timerRef.current = window.setTimeout(() => {
        lastStartedAtRef.current = performance.now();
        void lookUp(scanTarget);
      }, delay);
    }

    function onPointerMove(event: PointerEvent) {
      if (!isHeld() || !modifierMatchesEvent(modifier, event)) {
        return;
      }
      pointerRef.current = { x: event.clientX, y: event.clientY };
      if (!frameRef.current) {
        frameRef.current = true;
        requestAnimationFrame(processPointer);
      }
    }

    const keyName = modifierKeyName(modifier);

    function onKeyDown(event: KeyboardEvent) {
      if (event.key === keyName) {
        heldRef.current = true;
      }
      if (event.key === "Escape") {
        close();
      }
    }

    function onKeyUp(event: KeyboardEvent) {
      if (event.key !== keyName) {
        return;
      }
      heldRef.current = false;
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
        timerRef.current = null;
      }
      if (releaseBehavior === "close") {
        close();
      }
    }

    // A keyup swallowed by focus loss would otherwise leave scanning stuck on forever.
    function onBlur() {
      heldRef.current = false;
    }

    function onPointerDown(event: PointerEvent) {
      const inPopup = (event.target as Element | null)?.closest?.(
        ".anki-lookup-popup",
      );
      if (!inPopup) {
        close();
      }
    }

    document.addEventListener("pointermove", onPointerMove, true);
    document.addEventListener("keydown", onKeyDown, true);
    document.addEventListener("keyup", onKeyUp, true);
    document.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("blur", onBlur);
    return () => {
      document.removeEventListener("pointermove", onPointerMove, true);
      document.removeEventListener("keydown", onKeyDown, true);
      document.removeEventListener("keyup", onKeyUp, true);
      document.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("blur", onBlur);
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [modifier, releaseBehavior, debounceMs, heldOverride, enabled, close, lookUp]);

  // Releasing the modifier in "close" mode is handled above; this covers the overlay,
  // whose held state arrives from Rust rather than from a keyup.
  useEffect(() => {
    if (heldOverride === false && releaseBehavior === "close") {
      close();
    }
  }, [heldOverride, releaseBehavior, close]);

  return { target, result, isLoading, error, close };
}
