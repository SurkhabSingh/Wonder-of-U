import type { AnkiCatalog } from "../types";

/// The fields a catalog reports, or null when it never reached Anki.
///
/// `load_anki_catalog` treats a closed Anki as an ordinary state: it RESOLVES with
/// `status: "offline"` and an empty `fields`, rather than failing. Those empty fields
/// mean "nobody was asked", not "this note type has none", and the difference is not
/// cosmetic — a caller that caches the empty list both blanks its dropdown and, where
/// the cache key doubles as the "have we asked yet" gate, stops anything ever asking
/// again. Reading the payload through here is what keeps the two apart.
export function readyCatalogFields(catalog: AnkiCatalog): string[] | null {
  return catalog.status === "ready" ? catalog.fields : null;
}
