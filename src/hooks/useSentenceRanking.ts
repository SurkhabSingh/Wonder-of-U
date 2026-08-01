import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type { LineRanking, TranscriptRanking } from "../types";

/**
 * How many words in each visible line are still new.
 *
 * Keyed on the lines themselves rather than on a recording, because the rows on
 * screen are not always the rows on disk — merging or splitting a sentence changes
 * what needs ranking, and an index-keyed result would silently describe the
 * previous shape of the transcript.
 *
 * `builtAtMs` is the known-word list's timestamp, and it is in the dependencies so
 * that a Refresh re-ranks what is already open. Without it, the words you learned
 * this morning would not show up until the page was left and come back to.
 */
export function useSentenceRanking(
  lines: string[],
  builtAtMs: number | null,
): TranscriptRanking | null {
  const [ranking, setRanking] = useState<TranscriptRanking | null>(null);

  // Lines arrive as a fresh array every render, so the array itself cannot be a
  // dependency — it would re-rank on every keystroke in the search box. Its
  // contents are what actually matter.
  const linesKey = lines.join("\n");

  // Ranking a long episode is several hundred tokenizations, so a stale reply from
  // a previous transcript must never overwrite the current one. Each run claims the
  // token and only the newest may publish.
  const latestRun = useRef(0);

  useEffect(() => {
    if (lines.length === 0) {
      setRanking(null);
      return;
    }

    const run = latestRun.current + 1;
    latestRun.current = run;

    void (async () => {
      try {
        const result = await invoke<TranscriptRanking>("rank_transcript_lines", {
          lines,
        });
        if (latestRun.current === run) {
          setRanking(result);
        }
      } catch {
        // Ranking is an addition to the transcript, never a precondition for
        // reading it. A failure leaves the rows unbadged rather than taking the
        // page down with it.
        if (latestRun.current === run) {
          setRanking(null);
        }
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [linesKey, builtAtMs]);

  return ranking;
}

/**
 * Whether a line is worth mining: everything known but one word.
 *
 * Reads the flag rather than recomputing it. The rule has one home, in Rust, where
 * the summary count is also produced — two copies would drift, and the visible
 * symptom would be a toggle promising 242 lines and showing 143.
 */
export function isWithinReach(line: LineRanking): boolean {
  return line.withinReach;
}
