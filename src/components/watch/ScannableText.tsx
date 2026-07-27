import {
  SCANNABLE_CLASS,
  SCANNABLE_OWNER_ATTRIBUTE,
} from "../../hooks/useWordScanner";

// A line whose words can be looked up.
//
// It renders nothing but the text and two markers. All the behaviour lives in
// `useWordScanner`, which finds this element by walking up from the pointer — so a list of
// a thousand cues costs a thousand plain spans and not one event handler, and the matched
// word is drawn with the Custom Highlight API rather than by splitting the text into
// wrapper elements that would be rebuilt on every tick of the player's clock.
export function ScannableText({
  ownerKey,
  text,
}: {
  /// Identifies this line, so a popup can never highlight a line that has since been
  /// replaced by the video moving on.
  ownerKey: string;
  text: string;
}) {
  return (
    <span
      className={SCANNABLE_CLASS}
      {...{ [SCANNABLE_OWNER_ATTRIBUTE]: ownerKey }}
    >
      {text}
    </span>
  );
}
