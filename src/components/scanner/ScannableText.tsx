import type { ReactNode } from "react";
import {
  SCANNABLE_CLASS,
  SCANNABLE_END_ATTRIBUTE,
  SCANNABLE_OWNER_ATTRIBUTE,
  SCANNABLE_START_ATTRIBUTE,
} from "../../hooks/useWordScanner";

// Marks text whose words can be looked up.
//
// It renders nothing but its children and two markers. All the behaviour lives in
// `useWordScanner`, which finds this element by walking up from the pointer — so a list of
// a thousand rows costs a thousand plain spans and not one event handler.
//
// Children rather than a plain string, because the transcript viewer wraps search matches
// in `<mark>`. The scanner reads `textContent` and walks text nodes for its offsets, so
// nested markup is transparent to it, and the match itself is drawn with the Custom
// Highlight API rather than by adding wrappers of its own.
export function ScannableText({
  ownerKey,
  startMs,
  endMs,
  children,
}: {
  /// Identifies this line, so a popup can never highlight a line that has since been
  /// replaced — by the video moving on, or by scrolling a virtualised list.
  ownerKey: string;
  /// This line's moment in its recording, when it has one. Written as attributes
  /// rather than passed anywhere, because the scanner already walks up to this
  /// element to find the owner and can read them in the same step — the alternative
  /// was threading a resolver from the viewer up to the popup and back down.
  ///
  /// A line without them is simply not minable for a word: a translation row and a
  /// live transcript segment both have text and no clip behind it.
  startMs?: number | null;
  endMs?: number | null;
  children: ReactNode;
}) {
  const timed = typeof startMs === "number" && typeof endMs === "number";
  return (
    <span
      className={SCANNABLE_CLASS}
      {...{ [SCANNABLE_OWNER_ATTRIBUTE]: ownerKey }}
      {...(timed
        ? {
            [SCANNABLE_START_ATTRIBUTE]: String(startMs),
            [SCANNABLE_END_ATTRIBUTE]: String(endMs),
          }
        : {})}
    >
      {children}
    </span>
  );
}
