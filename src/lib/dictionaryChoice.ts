import type { LookupDictionaries, LookupDictionary } from "../types";

// Reading the add-on's dictionary listing.
export function installedDictionaries(
  listing: LookupDictionaries | null,
): LookupDictionary[] | null {
  return listing?.status === "ready" ? listing.dictionaries : null;
}

/// Chosen ids the add-on no longer lists.
export function missingDictionaryIds(
  installed: LookupDictionary[] | null,
  chosenIds: number[],
): number[] {
  if (installed === null) {
    return [];
  }
  return chosenIds.filter((id) => !installed.some((entry) => entry.id === id));
}

/// Shown when the listing was rejected outright rather than answered.
export const DICTIONARIES_UNREADABLE =
  "Your dictionaries couldn't be listed. Restart Anki and try again.";
