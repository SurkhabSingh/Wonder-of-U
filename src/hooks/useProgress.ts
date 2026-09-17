import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { PROGRESS_EVENT } from "../constants";
import type { Measured, ProgressReport } from "../types";

// Two writes that land together, one from each sampler, are one reload.
const SETTLE_MS = 400;

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
  // One guard each: a live reload of the report must not discard a card count still on
  // its way back from Anki.
  const reportRun = useRef(0);
  const cardsRun = useRef(0);

  const loadReport = useCallback(async () => {
    const run = reportRun.current + 1;
    reportRun.current = run;
    try {
      const next = await invoke<ProgressReport>("load_progress");
      if (reportRun.current !== run) {
        return;
      }
      setReport(next);
      setFailed(false);
      setReadCount((count) => count + 1);
    } catch {
      if (reportRun.current === run) {
        setFailed(true);
      }
    }
  }, []);

  const loadCards = useCallback(async () => {
    const run = cardsRun.current + 1;
    cardsRun.current = run;
    try {
      const counted = await invoke<Measured<number>>("count_mined_cards");
      if (cardsRun.current === run) {
        setMinedCards(counted);
      }
    } catch {
      if (cardsRun.current === run) {
        setMinedCards({
          value: null,
          status: "unavailable",
          asOfMs: null,
          reason: "The cards in your collection could not be counted.",
        });
      }
    }
  }, []);

  const refresh = useCallback(async () => {
    await loadReport();
    // After the report, so a slow Anki never holds up the local numbers.
    await loadCards();
  }, [loadReport, loadCards]);

  useEffect(() => {
    if (activePage !== "progress") {
      return;
    }
    void refresh();

    let settle: number | undefined;
    const unlisten = listen(PROGRESS_EVENT, () => {
      window.clearTimeout(settle);
      settle = window.setTimeout(() => void loadReport(), SETTLE_MS);
    });
    return () => {
      window.clearTimeout(settle);
      void unlisten.then((stop) => stop());
    };
  }, [activePage, refresh, loadReport]);

  return { report, readCount, failed, minedCards, refreshProgress: refresh };
}
