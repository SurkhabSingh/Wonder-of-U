import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Measured, ProgressReport } from "../types";

/// Reads the stored progress report.
///
/// The command touches local files only and never contacts Anki, so this is cheap and can
/// run whenever the page mounts. It also never TAKES a reading — the only thing that moves
/// coverage is rebuilding the word list, so a reading is taken there and read here.
export function useProgress(activePage: string) {
  const [report, setReport] = useState<ProgressReport | null>(null);
  /// How many times a read has SUCCEEDED, as a counter rather than a boolean.
  ///
  /// The page has to tell "we have looked and there is nothing stored" from "we have never
  /// managed to look", and a null `report` means both. Copied from `useMinedSentences`,
  /// which needed the same distinction for the same reason.
  const [readCount, setReadCount] = useState(0);
  const [failed, setFailed] = useState(false);
  // Asked for separately from the report, because it is the one number that needs Anki and
  // the page must open at the same speed whether Anki is running or not. Null means the
  // question has not been put yet, which is not the same as Anki having declined it.
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
      // The command rejected rather than answering. Its own wording is not shown — every
      // reason worth telling apart is already a state inside the report — so the page says
      // the one fixed thing instead.
      if (runRef.current === run) {
        setFailed(true);
      }
    }

    // Fired after the report and never awaited with it: a slow or absent Anki must not be
    // able to hold up the numbers that came from local files.
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

  // Read on arrival, not on a timer: nothing on this page changes while it is open, since
  // a reading is only taken where the word list is rebuilt.
  useEffect(() => {
    if (activePage !== "progress") {
      return;
    }
    void refresh();
  }, [activePage, refresh]);

  return { report, readCount, failed, minedCards, refreshProgress: refresh };
}
