import { useCallback, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { MinedSentences } from "../types";

// The sentences already sitting in the Anki mining destination. Unlike the Anki
// catalog this does NOT poll: a full read of the deck is far too heavy for a 10s
// interval, and the set only changes when the user mines. Callers refresh it when the
// transcript viewer opens, when Anki becomes reachable, and after a successful mine.
export function useMinedSentences() {
  const [minedSentences, setMinedSentences] = useState<Set<string>>(
    () => new Set(),
  );
  // A mapping that points at a deck, note type or field Anki no longer has. Distinct from
  // "offline" and "nothing configured yet", which are both quiet on purpose — this one the
  // user can fix, and without it the only symptom is marks that never appear.
  const [minedWarning, setMinedWarning] = useState<string | null>(null);
  // When the deck was last read SUCCESSFULLY, as a counter rather than a boolean.
  const [readCount, setReadCount] = useState(0);
  const inFlightRef = useRef(false);
  const pendingRef = useRef(false);

  const refreshMinedSentences = useCallback(async () => {
    if (inFlightRef.current) {
      pendingRef.current = true;
      return;
    }
    inFlightRef.current = true;
    try {
      do {
        pendingRef.current = false;
        const result = await invoke<MinedSentences>("load_mined_sentences");
        if (result.status === "ready") {
          setMinedSentences(new Set(result.sentences));
          setMinedWarning(null);
          setReadCount((previous) => previous + 1);
        } else if (result.status === "stale") {
          setMinedWarning(result.message);
        } else if (import.meta.env.DEV) {
          console.debug("load_mined_sentences:", result.status, result.message);
        }
      } while (pendingRef.current);
    } catch (error) {
      if (import.meta.env.DEV) {
        console.debug("load_mined_sentences failed:", error);
      }
    } finally {
      inFlightRef.current = false;
    }
  }, []);

  return {
    minedSentences,
    minedWarning,
    // Only ever incremented on a `ready` read, so `0` means the deck has never been seen.
    minedReadCount: readCount,
    refreshMinedSentences,
  };
}
