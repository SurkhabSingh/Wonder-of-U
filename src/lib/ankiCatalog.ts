import type { AnkiCatalog } from "../types";

/// The fields of `noteType`, or null when they are not known to be its fields.
///
/// Null covers three situations that all mean the same thing to a caller — draw no
/// conclusions — and they arrive by different routes:
///
///   * Anki was never reached. `load_anki_catalog` treats a closed Anki as an ordinary
///     state and RESOLVES with `status: "offline"`, so the absence has to be read off
///     the status rather than off a rejection.
///   * the catalog describes a different note type. A refresh leaves the previous one in
///     place while it runs, so between choosing a note type and the answer arriving,
///     `fields` still holds the previous note type's names.
///   * Anki has no note type by that name.
///
/// The middle one is the easy one to miss and the worst to get wrong: anything that
/// compared a mapping against a stale catalog would name the note type on screen while
/// describing the fields of the one before it, and be wrong in every noun.
export function fieldsForNoteType(
  catalog: AnkiCatalog,
  noteType: string,
): string[] | null {
  if (catalog.status !== "ready" || noteType === "") {
    return null;
  }
  return catalog.noteType === noteType ? catalog.fields : null;
}

/// Whether Anki was asked for this note type and had none by that name.
///
/// Deliberately not `noteTypes.includes()`. The displayed catalog has the saved note type
/// merged into that list so the picker keeps showing what is selected, which makes the
/// list useless for deciding whether Anki actually has it. `fields: null` on the note type
/// the catalog was asked about is Anki's own answer, and it survives the merge.
///
/// Guarded on the echoed name for the same reason as `fieldsForNoteType`: mid-refresh the
/// catalog still describes the previous note type, and reading it then would report a note
/// type as gone a moment after it was chosen.
export function noteTypeMissingFromAnki(
  catalog: AnkiCatalog,
  noteType: string,
): boolean {
  return (
    catalog.status === "ready" &&
    noteType !== "" &&
    catalog.noteType === noteType &&
    catalog.fields === null
  );
}
