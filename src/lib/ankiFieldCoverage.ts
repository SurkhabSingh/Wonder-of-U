import type { AnkiFieldMapping } from "../types";

/// How the saved mapping lines up with the note type it points at.
export type AnkiFieldCoverage = {
  missing: string[];
  unfilled: string[];
  emptyFirstField: string | null;
};

/// Compares the saved mapping against the fields the note type actually has.
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
    unfilled: fields.filter((name) => !mapped.has(name)),
    emptyFirstField: first !== undefined && !mapped.has(first) ? first : null,
  };
}
