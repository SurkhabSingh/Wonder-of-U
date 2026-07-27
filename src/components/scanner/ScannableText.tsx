import type { ReactNode } from "react";
import {
  SCANNABLE_CLASS,
  SCANNABLE_OWNER_ATTRIBUTE,
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
  children,
}: {
  /// Identifies this line, so a popup can never highlight a line that has since been
  /// replaced — by the video moving on, or by scrolling a virtualised list.
  ownerKey: string;
  children: ReactNode;
}) {
  return (
    <span
      className={SCANNABLE_CLASS}
      {...{ [SCANNABLE_OWNER_ATTRIBUTE]: ownerKey }}
    >
      {children}
    </span>
  );
}
