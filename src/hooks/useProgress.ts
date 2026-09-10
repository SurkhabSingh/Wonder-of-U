import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ProgressReport } from "../types";

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
  }, []);

  // Read on arrival, not on a timer: nothing on this page changes while it is open, since
  // a reading is only taken where the word list is rebuilt.
  useEffect(() => {
    if (activePage !== "progress") {
      return;
    }
    void refresh();
  }, [activePage, refresh]);

  return { report, readCount, failed, refreshProgress: refresh };
}
