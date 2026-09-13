import type { AnkiCatalog } from "../types";

/// The fields of `noteType`, or null when they are not known to be its fields.
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
