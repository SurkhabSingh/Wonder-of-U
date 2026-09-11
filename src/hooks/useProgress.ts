import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Measured, ProgressReport } from "../types";

/// Reads the stored report. Local files only, and never takes a reading: coverage moves
/// when the word list is rebuilt, so a reading is taken there and read here.
export function useProgress(activePage: string) {
  const [report, setReport] = useState<ProgressReport | null>(null);
  /// Successful reads. A null `report` means both "nothing stored" and "never looked",
  /// and the page needs to tell them apart.
  const [readCount, setReadCount] = useState(0);
  const [failed, setFailed] = useState(false);
  // Separate, because it is the one number needing Anki. Null is "not asked yet", which
  // is not the same as Anki declining.
  const [minedCards, setMinedCards] = useState<Measured<number> | null>(null);
  /// Guards two answers to two requests landing out of order.
  const runRef = useRef(0);

  const refresh = useCallback(async () => {
    const run = runRef.current + 1;
    runRef.current = run;
    try {
      const next = await invoke<ProgressReport>("load_progress");
      if (runRef.current !== run) {
        return;
      }
      setReport(next);
      setFailed(false);
      setReadCount((count) => count + 1);
    } catch {
      if (runRef.current === run) {
        setFailed(true);
      }
    }

    // Never awaited with the report: a slow Anki must not hold up local numbers.
    try {
      const counted = await invoke<Measured<number>>("count_mined_cards");
      if (runRef.current === run) {
        setMinedCards(counted);
      }
    } catch {
      if (runRef.current === run) {
        setMinedCards({
          value: null,
          status: "unavailable",
          asOfMs: null,
          reason: "The cards in your collection could not be counted.",
        });
      }
    }
  }, []);

  useEffect(() => {
    if (activePage !== "progress") {
      return;
    }
    void refresh();
  }, [activePage, refresh]);

  return { report, readCount, failed, minedCards, refreshProgress: refresh };
}
