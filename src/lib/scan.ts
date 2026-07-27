// Finding which character of a subtitle line the pointer is on.
//
// The obvious implementation — one `<span>` per character carrying its index — is
// rejected on cost. The subtitle list renders every cue in the file (400-1500 rows) and
// re-renders on the player's 250 ms poll; a span per character would multiply that DOM by
// the length of a line. The caret APIs answer the same question against a single text
// node, so scanning adds no elements at all until something is actually matched.

/// JavaScript counts UTF-16 code units; Rust's `chars()` counts code points. They agree on
/// every character in ordinary Japanese, and disagree on the ones outside the BMP — 𠮟る,
/// 𩸽, emoji — where a DOM offset would land the backend a character to the right. Convert
/// before crossing the boundary rather than hoping subtitles stay inside the BMP.
export function codePointOffset(text: string, utf16Offset: number): number {
  let index = 0;
  let position = 0;
  while (position < utf16Offset && position < text.length) {
    const code = text.codePointAt(position);
    position += code !== undefined && code > 0xffff ? 2 : 1;
    index += 1;
  }
  return index;
}

/// `caretRangeFromPoint` is the Blink spelling and is what WebView2 has;
/// `caretPositionFromPoint` is the standard one. Try both rather than assuming.
function caretRangeAt(x: number, y: number): { node: Node; offset: number } | null {
  const legacy = (
    document as Document & {
      caretRangeFromPoint?: (x: number, y: number) => Range | null;
    }
  ).caretRangeFromPoint;
  if (typeof legacy === "function") {
    const range = legacy.call(document, x, y);
    if (range) {
      return { node: range.startContainer, offset: range.startOffset };
    }
  }
  const standard = (
    document as Document & {
      caretPositionFromPoint?: (
        x: number,
        y: number,
      ) => { offsetNode: Node; offset: number } | null;
    }
  ).caretPositionFromPoint;
  if (typeof standard === "function") {
    const position = standard.call(document, x, y);
    if (position) {
      return { node: position.offsetNode, offset: position.offset };
    }
  }
  return null;
}

function textNodesOf(container: HTMLElement): Text[] {
  const walker = document.createTreeWalker(container, NodeFilter.SHOW_TEXT);
  const nodes: Text[] = [];
  let node = walker.nextNode();
  while (node) {
    nodes.push(node as Text);
    node = walker.nextNode();
  }
  return nodes;
}

/// Total text length of the container, which is the string the offsets index into.
export function scanTextLength(container: HTMLElement): number {
  return textNodesOf(container).reduce(
    (total, node) => total + node.data.length,
    0,
  );
}

/// Container-wide offset of a position inside one of its text nodes.
function flattenOffset(
  container: HTMLElement,
  node: Node,
  offsetInNode: number,
): number | null {
  let total = 0;
  for (const text of textNodesOf(container)) {
    if (text === node) {
      return total + offsetInNode;
    }
    total += text.data.length;
  }
  return null;
}

/// The reverse: a container-wide offset back to a (text node, offset) pair.
function locate(
  container: HTMLElement,
  offset: number,
): { node: Text; offset: number } | null {
  let remaining = offset;
  for (const text of textNodesOf(container)) {
    if (remaining < text.data.length) {
      return { node: text, offset: remaining };
    }
    remaining -= text.data.length;
  }
  return null;
}

/// A DOMRect for the single character at `offset`, or null if there is none.
export function characterRect(
  container: HTMLElement,
  offset: number,
): DOMRect | null {
  const start = locate(container, offset);
  const end = locate(container, offset + 1) ?? {
    node: start?.node as Text,
    offset: (start?.offset ?? 0) + 1,
  };
  if (!start || !end.node) {
    return null;
  }
  const range = document.createRange();
  try {
    range.setStart(start.node, start.offset);
    range.setEnd(end.node, end.offset);
  } catch {
    // An offset past the end of the node — nothing to measure.
    return null;
  }
  const rects = range.getClientRects();
  return rects.length > 0 ? rects[0] : null;
}

function rectHasPoint(rect: DOMRect | null, x: number, y: number): boolean {
  return (
    rect !== null &&
    x >= rect.left &&
    x <= rect.right &&
    y >= rect.top &&
    y <= rect.bottom
  );
}

/// The index of the character under the pointer, or null if the pointer is not on text.
///
/// The caret APIs snap to the nearest position *between* characters, so a click on the
/// right half of a character reports the offset after it — which would look up the next
/// word. Rather than trusting the snap, measure the character it names and its neighbour
/// and keep whichever actually contains the point.
export function characterOffsetFromPoint(
  container: HTMLElement,
  x: number,
  y: number,
): number | null {
  const caret = caretRangeAt(x, y);
  if (!caret || !container.contains(caret.node)) {
    return null;
  }
  const flattened = flattenOffset(container, caret.node, caret.offset);
  if (flattened === null) {
    return null;
  }
  if (rectHasPoint(characterRect(container, flattened), x, y)) {
    return flattened;
  }
  if (
    flattened > 0 &&
    rectHasPoint(characterRect(container, flattened - 1), x, y)
  ) {
    return flattened - 1;
  }
  // Clicked past the end of a line: fall back to the last character rather than nothing,
  // so a click near the right edge still scans the word there.
  const length = scanTextLength(container);
  if (length === 0) {
    return null;
  }
  return Math.min(flattened, length - 1);
}
