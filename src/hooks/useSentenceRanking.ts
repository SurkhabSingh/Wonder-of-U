import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type { LineRanking, TranscriptRanking } from "../types";

// How many words in each visible line are still new.

export function useSentenceRanking(
  lines: string[],
  builtAtMs: number | null,
): TranscriptRanking | null {
  const [ranking, setRanking] = useState<TranscriptRanking | null>(null);

  // Lines arrive as a fresh array every render, so the array itself cannot be a
  // dependency — it would re-rank on every keystroke in the search box. Its
  // contents are what actually matter.
  const linesKey = lines.join("\n");

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
        const result = await invoke<TranscriptRanking>(
          "rank_transcript_lines",
          {
            lines,
          },
        );
        if (latestRun.current === run) {
          setRanking(result);
        }
      } catch {
        if (latestRun.current === run) {
          setRanking(null);
        }
      }
    })();
  }, [linesKey, builtAtMs]);

  return ranking;
}

// Whether a line is one word away: everything known but one.

export function isWithinReach(line: LineRanking): boolean {
  return line.withinReach;
}

export function isBareWord(line: LineRanking): boolean {
  return line.withinReach && !line.hasContext;
}
