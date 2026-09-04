import type { LookupDictionaries, LookupDictionary } from "../types";

// Reading the add-on's dictionary listing.
//
// The listing answers two different questions with the same shape, and the whole of
// this file exists to keep them apart: "here is what is installed" and "nobody
// answered" both arrive as an object carrying a `dictionaries` array. An unavailable
// listing carries an EMPTY one — because Anki was closed, still starting, or too busy
// to reply inside the timeout — not because the dictionaries were removed.

/// What the add-on reports it has, or null when it did not report at all.
///
/// Only a `ready` listing knows what is installed. Every read of the list goes through
/// here so the distinction cannot be honoured at one call site and forgotten at
/// another — which is how a settings page came to tell users their dictionaries had
/// been uninstalled every time Anki happened to be shut.
export function installedDictionaries(
  listing: LookupDictionaries | null,
): LookupDictionary[] | null {
  return listing?.status === "ready" ? listing.dictionaries : null;
}

/// Chosen ids the add-on no longer lists.
///
/// Empty whenever the listing is not authoritative: absence of an answer is not
/// evidence of absence. This matters more than a wrong label, because the warning it
/// drives offers a one-click "forget the missing ones" that rewrites the user's
/// saved choice — so a false positive here silently erases real settings.
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
///
/// Fixed copy, deliberately: a rejection carries the backend's own words — a parse
/// error, a reqwest internal, the add-on's Python exception text — and this string is
/// rendered as the section's only sentence. The reasons worth telling apart are named
/// in Rust, where they can be identified; anything reaching here is one the app cannot
/// explain, so it says the one useful thing instead.
export const DICTIONARIES_UNREADABLE =
  "Your dictionaries couldn't be listed. Restart Anki and try again.";
