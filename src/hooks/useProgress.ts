import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { CARD_MADE_EVENT, PROGRESS_EVENT } from "../constants";
import type { Measured, ProgressReport } from "../types";

// Two writes that land together, one from each sampler, are one reload.
const SETTLE_MS = 400;
// Long enough that a run of cards pushed together is counted once, after the last.
const RECOUNT_MS = 1500;

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
  const [countingCards, setCountingCards] = useState(false);
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
    setCountingCards(true);
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
    } finally {
      if (cardsRun.current === run) {
        setCountingCards(false);
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
    let recount: number | undefined;
    // A count announces a change of its own, so a change starting one would loop.
    const unlisteners = [
      listen(PROGRESS_EVENT, () => {
        window.clearTimeout(settle);
        settle = window.setTimeout(() => void loadReport(), SETTLE_MS);
      }),
      listen(CARD_MADE_EVENT, () => {
        window.clearTimeout(recount);
        recount = window.setTimeout(() => void loadCards(), RECOUNT_MS);
      }),
    ];
    return () => {
      window.clearTimeout(settle);
      window.clearTimeout(recount);
      for (const unlisten of unlisteners) {
        void unlisten.then((stop) => stop());
      }
    };
  }, [activePage, refresh, loadReport, loadCards]);

  return {
    report,
    readCount,
    failed,
    minedCards,
    countingCards,
    countCards: loadCards,
    refreshProgress: refresh,
  };
}
