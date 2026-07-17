import { useCallback, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { DEFAULT_KNOWN_WORDS } from "../constants";
import type { BusyAction, KnownWordsSnapshot } from "../types";
import { errorMessage } from "../lib/errors";

type UseKnownWordsOptions = {
  persistSettingsIfNeeded: () => Promise<void>;
  setBusyAction: (busyAction: BusyAction) => void;
  showError: (message: string) => void;
  showSuccess: (message: string) => void;
  showWarning: (message: string) => void;
};

// Refreshing is manual and stays manual. Anki is edited outside this app and
// AnkiConnect cannot tell us when that happens, so there is no moment we could
// honestly call the index stale — a timer would either walk the user's whole
// collection on a schedule or lie with equal confidence either way. The snapshot
// carries the build time so the user can judge it themselves.
export function useKnownWords({
  persistSettingsIfNeeded,
  setBusyAction,
  showError,
  showSuccess,
  showWarning,
}: UseKnownWordsOptions) {
  const [knownWords, setKnownWords] =
    useState<KnownWordsSnapshot>(DEFAULT_KNOWN_WORDS);

  const refreshKnownWords = useCallback(async () => {
    try {
      setBusyAction("knownWords");
      // The backend reads the note type and field off the persisted settings, so
      // a draft the user has not saved yet would build the previous selection.
      await persistSettingsIfNeeded();
      const snapshot = await invoke<KnownWordsSnapshot>("refresh_known_words");
      setKnownWords(snapshot);

      if (snapshot.status === "ready") {
        showSuccess(snapshot.message);
      } else if (snapshot.status !== "unconfigured") {
        // Offline and empty are both states the card explains in place; the toast
        // is only here so a press of the button is never silent.
        showWarning(snapshot.message);
      }
    } catch (error) {
      showError(
        errorMessage(error, "Your known words could not be read from Anki."),
      );
    } finally {
      setBusyAction(null);
    }
  }, [
    persistSettingsIfNeeded,
    setBusyAction,
    showError,
    showSuccess,
    showWarning,
  ]);

  return {
    knownWords,
    refreshKnownWords,
  };
}
