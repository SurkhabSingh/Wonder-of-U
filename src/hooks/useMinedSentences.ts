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
  const inFlightRef = useRef(false);
  // A refresh asked for while another was running. Dropping it outright would lose
  // the update for good — there is no poll to retry it — so the request is remembered
  // and replayed once, which still keeps at most one read of the deck in flight.
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
        // Only a successful read describes the deck. "offline" / "unmapped" / "error"
        // all carry an empty list that means "could not look", not "you have mined
        // nothing" — overwriting with it would wipe correct marks the moment Anki
        // closed, and quietly claim every sentence was unmined.
        if (result.status === "ready") {
          setMinedSentences(new Set(result.sentences));
          setMinedWarning(null);
        } else if (result.status === "stale") {
          setMinedWarning(result.message);
        } else if (import.meta.env.DEV) {
          console.debug("load_mined_sentences:", result.status, result.message);
        }
      } while (pendingRef.current);
    } catch (error) {
      // Marking sentences is an enhancement, so a failure stays silent: mining itself
      // already reports why it cannot run, and this must never block reading a
      // transcript. The backend degrades to an Ok status for offline/unmapped, so
      // reaching here means the IPC call itself failed.
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
    refreshMinedSentences,
  };
}
