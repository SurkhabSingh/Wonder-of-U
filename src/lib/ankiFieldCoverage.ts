import type { AnkiFieldMapping } from "../types";

/// How the saved mapping lines up with the note type it points at.
export type AnkiFieldCoverage = {
  /// Mapped names the note type does not have.
  ///
  /// These are the dangerous half. AnkiConnect drops a write to a field name the
  /// note type lacks and says nothing, so the card arrives missing that content and
  /// looks fine. Only a mapping where EVERY name is wrong ever surfaces, because
  /// Anki then refuses the note as empty — one wrong name just goes quiet.
  missing: string[];
  /// Fields the note type has that nothing writes to.
  ///
  /// Not a fault: this app produces twelve kinds of content, and a community note
  /// type built for reading can carry twenty or more. Listed so the gap is visible
  /// rather than guessed at, because nothing else on the page names it.
  unfilled: string[];
  /// The note type's FIRST field, when nothing writes to it.
  ///
  /// Not a gap in coverage — a push that cannot happen at all. Anki refuses any note
  /// whose first field is blank, so every card fails while this is set, and the error
  /// it fails with says the card was "empty" without saying which field it means.
  ///
  /// Our own note type puts the transcript first, so it never trips. A note type
  /// built for reading usually starts with the word being studied, which nothing here
  /// writes — which is exactly how choosing one silently guarantees failure.
  emptyFirstField: string | null;
};

/// Compares the saved mapping against the fields the note type actually has.
///
/// `fields` is null when the note type's field list is not known — Anki closed, or
/// the answer not back yet. Returns null in that case rather than reporting every
/// mapped name as missing: absence of an answer is not evidence of absence, and this
/// one drives a warning about the user's own settings.
export function ankiFieldCoverage(
  fields: string[] | null,
  mapping: AnkiFieldMapping,
): AnkiFieldCoverage | null {
  if (fields === null) {
    return null;
  }

  const available = new Set(fields);
  const mapped = new Set(Object.values(mapping).filter((name) => name !== ""));

  const [first] = fields;

  return {
    missing: [...mapped].filter((name) => !available.has(name)),
    // Kept in the note type's own order, which is the order Anki shows them in, so
    // the list reads against what the user sees in the card editor.
    unfilled: fields.filter((name) => !mapped.has(name)),
    emptyFirstField: first !== undefined && !mapped.has(first) ? first : null,
  };
}
