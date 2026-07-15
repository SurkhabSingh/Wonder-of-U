import { useCallback, useEffect, useRef, useState } from "react";
import { emit, listen } from "@tauri-apps/api/event";
import { fileNameFromPath } from "../lib/format";
import type { RecordingBatchResult, YoutubeQueueItem } from "../types";

type UseYoutubeQueueOptions = {
  // Single-URL backend import. It BLOCKS until the download finishes and
  // resolves with the single-item batch (or a "failed"/"cancelled" batch). That
  // promise resolving IS the completion signal — we never wait on a progress
  // value or a "done" event. Returns null only when the fetch could not run at
  // all (thrown/cancelled).
  importYoutube: (url: string) => Promise<RecordingBatchResult | null>;
  // Fired once when the whole queue drains from busy → idle, with the number of
  // items that actually landed a recording. Lets the caller defer navigation.
  onAllComplete: (landedCount: number) => void;
};

// The app is single-flight on the shared download slot, so this is a strictly
// sequential queue on top of the single-URL backend import: only ever one
// `active` item, the rest wait as `queued`. Splitting a paste of many links
// into many queued items is what turns one download slot into a batch.
//
// Completion is the awaited `importYoutube` resolving — nothing else. Progress
// is a lightweight `youtube-progress` Tauri event (a percent number), and
// cancel is a `youtube-cancel` event that makes the active download resolve
// early as cancelled/failed so the loop moves on.
export function useYoutubeQueue({
  importYoutube,
  onAllComplete,
}: UseYoutubeQueueOptions) {
  const [items, setItems] = useState<YoutubeQueueItem[]>([]);
  // Percent for the single active download, or null when nothing is active.
  // Single-flight, so this always belongs to the one `active` item.
  const [activeProgress, setActiveProgress] = useState<number | null>(null);

  // Keep the injected callbacks in refs so the processor never captures a stale
  // closure and the effects don't re-fire because a parent re-rendered.
  const importYoutubeRef = useRef(importYoutube);
  const onAllCompleteRef = useRef(onAllComplete);
  importYoutubeRef.current = importYoutube;
  onAllCompleteRef.current = onAllComplete;

  // Mirror of `items` the async loop reads between iterations without capturing
  // a stale render closure. Kept in sync by the effect below.
  const itemsRef = useRef<YoutubeQueueItem[]>(items);
  // Guards: `runningRef` keeps the processor from running twice; `mountedRef`
  // stops setState after unmount.
  const runningRef = useRef(false);
  const mountedRef = useRef(true);
  // Stable id source — no crypto dependency, monotonic per session.
  const idRef = useRef(0);
  // Landed count + a busy latch, so `onAllComplete` fires exactly once per run.
  const landedRef = useRef(0);
  const wasBusyRef = useRef(false);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // Keep the ref in lock-step with state so the loop's next-item lookup and the
  // busy latch always read the latest queue.
  useEffect(() => {
    itemsRef.current = items;
  }, [items]);

  // Progress is a fire-and-forget percent event during the active download.
  useEffect(() => {
    const unlisten = listen<number>("youtube-progress", ({ payload }) => {
      if (!mountedRef.current) {
        return;
      }
      setActiveProgress(payload);
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  const enqueue = useCallback((text: string) => {
    // One paste of many links becomes many items; a single link is a queue of
    // one. Newlines, spaces/tabs, and commas all separate (URLs have none).
    const urls = text
      .split(/[\s,]+/)
      .map((part) => part.trim())
      .filter((part) => part.length > 0);
    if (urls.length === 0) {
      return;
    }
    setItems((prev) => {
      // Dedupe against still-pending/active items and within this paste.
      const seen = new Set(
        prev
          .filter((item) => item.status === "queued" || item.status === "active")
          .map((item) => item.url),
      );
      const additions: YoutubeQueueItem[] = [];
      for (const url of urls) {
        if (seen.has(url)) {
          continue;
        }
        seen.add(url);
        idRef.current += 1;
        additions.push({
          id: `yt-${idRef.current}`,
          url,
          status: "queued",
        });
      }
      if (additions.length === 0) {
        return prev;
      }
      return [...prev, ...additions];
    });
  }, []);

  const remove = useCallback((id: string) => {
    // Only a still-queued row can be dropped; active/terminal rows are a no-op.
    setItems((prev) =>
      prev.filter((item) => !(item.id === id && item.status === "queued")),
    );
  }, []);

  const cancelActive = useCallback(() => {
    // Cancel the active download slot. The active item's `importYoutube` promise
    // then resolves early (cancelled → null, or a failed batch), and the loop
    // advances to the next queued item.
    void emit("youtube-cancel");
  }, []);

  const clearFinished = useCallback(() => {
    setItems((prev) =>
      prev.filter(
        (item) => item.status === "queued" || item.status === "active",
      ),
    );
  }, []);

  // The sequential processor — a plain loop, mirroring vibe's batch `start()`.
  // Guarded by `runningRef` so it never runs twice. Each iteration promotes the
  // next queued item, awaits the blocking import to completion, and stamps the
  // terminal status from the RESOLVED result. Both the success and the error
  // path advance, so one failed item never blocks the rest of the queue.
  const startProcessing = useCallback(() => {
    if (runningRef.current) {
      return;
    }
    if (!itemsRef.current.some((item) => item.status === "queued")) {
      return;
    }
    runningRef.current = true;

    void (async () => {
      while (mountedRef.current) {
        const next = itemsRef.current.find((item) => item.status === "queued");
        if (!next) {
          break;
        }

        setItems((prev) =>
          prev.map((item) =>
            item.id === next.id ? { ...item, status: "active" } : item,
          ),
        );
        setActiveProgress(0);

        // Completion = this awaited invoke resolving. A throw (or a hard cancel)
        // surfaces as null; the loop still advances.
        let result: RecordingBatchResult | null = null;
        try {
          result = await importYoutubeRef.current(next.url);
        } catch {
          result = null;
        }

        if (!mountedRef.current) {
          break;
        }

        setItems((prev) =>
          prev.map((item) => {
            if (item.id !== next.id) {
              return item;
            }
            // null → the fetch could not run (most likely a user cancel).
            if (result === null) {
              return { ...item, status: "cancelled" };
            }
            const landed = result.items.find(
              (entry) => entry.status === "success",
            );
            if (landed) {
              landedRef.current += 1;
              return {
                ...item,
                status: "done",
                title: fileNameFromPath(landed.filePath),
              };
            }
            // A result with nothing landed is a real failure — keep the message.
            return { ...item, status: "failed", message: result.message };
          }),
        );
        setActiveProgress(null);
      }

      setActiveProgress(null);
      runningRef.current = false;
    })();
  }, []);

  // Kick the processor whenever a queued item appears and it isn't already
  // running. It self-guards on `runningRef`, so re-firing mid-run is a no-op.
  useEffect(() => {
    if (items.some((item) => item.status === "queued")) {
      startProcessing();
    }
  }, [items, startProcessing]);

  // Fire `onAllComplete` exactly once when the queue drains from busy → idle.
  useEffect(() => {
    const busy = items.some(
      (item) => item.status === "active" || item.status === "queued",
    );
    if (busy) {
      wasBusyRef.current = true;
      return;
    }
    if (wasBusyRef.current) {
      wasBusyRef.current = false;
      const landed = landedRef.current;
      landedRef.current = 0;
      onAllCompleteRef.current(landed);
    }
  }, [items]);

  const activeCount = items.filter((item) => item.status === "active").length;
  const queuedCount = items.filter((item) => item.status === "queued").length;
  const finishedCount = items.filter(
    (item) =>
      item.status === "done" ||
      item.status === "failed" ||
      item.status === "cancelled",
  ).length;

  return {
    items,
    enqueue,
    remove,
    cancelActive,
    clearFinished,
    activeProgress,
    activeCount,
    queuedCount,
    finishedCount,
    // 1-based position of the item being fetched, and the run total. Drive a
    // "Fetching N of M…" line: N = finished + active, M = items.length.
    currentIndex: finishedCount + activeCount,
    total: items.length,
  };
}
