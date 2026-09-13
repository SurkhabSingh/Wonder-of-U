import { useCallback, useEffect, useRef, useState } from "react";
import { emit, listen } from "@tauri-apps/api/event";
import { errorMessage } from "../lib/errors";
import { fileNameFromPath } from "../lib/format";
import type { YoutubeImportOutcome, YoutubeQueueItem } from "../types";

type UseYoutubeQueueOptions = {
  importYoutube: (url: string) => Promise<YoutubeImportOutcome>;
  onAllComplete: (landedCount: number) => void;
};

function splitPastedUrls(text: string): string[] {
  return text
    .split(/\s+/)
    .flatMap((token) => token.split(/,(?=https?:\/\/)/))
    .map((part) => part.replace(/,+$/, "").trim())
    .filter((part) => part.length > 0);
}

export function useYoutubeQueue({
  importYoutube,
  onAllComplete,
}: UseYoutubeQueueOptions) {
  const [items, setItems] = useState<YoutubeQueueItem[]>([]);
  const [activeProgress, setActiveProgress] = useState<number | null>(null);

  const importYoutubeRef = useRef(importYoutube);
  const onAllCompleteRef = useRef(onAllComplete);
  importYoutubeRef.current = importYoutube;
  onAllCompleteRef.current = onAllComplete;

  const itemsRef = useRef<YoutubeQueueItem[]>(items);
  const runningRef = useRef(false);
  const mountedRef = useRef(true);
  const idRef = useRef(0);
  const landedRef = useRef(0);
  const wasBusyRef = useRef(false);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

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
    // one.
    const urls = splitPastedUrls(text);
    if (urls.length === 0) {
      return;
    }
    setItems((prev) => {
      // Dedupe against still-pending/active items and within this paste.
      const seen = new Set(
        prev
          .filter(
            (item) => item.status === "queued" || item.status === "active",
          )
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
  const startProcessing = useCallback(() => {
    if (runningRef.current) {
      return;
    }
    if (!itemsRef.current.some((item) => item.status === "queued")) {
      return;
    }
    runningRef.current = true;

    void (async () => {
      try {
        while (mountedRef.current) {
          const next = itemsRef.current.find(
            (item) => item.status === "queued",
          );
          if (!next) {
            break;
          }

          setItems((prev) =>
            prev.map((item) =>
              item.id === next.id ? { ...item, status: "active" } : item,
            ),
          );
          setActiveProgress(0);

          let settled: YoutubeImportOutcome;
          try {
            settled = await importYoutubeRef.current(next.url);
          } catch (error) {
            settled = {
              ok: false,
              message: errorMessage(
                error,
                "The YouTube link could not be imported.",
              ),
            };
          }
          const outcome = settled;

          if (!mountedRef.current) {
            break;
          }

          setItems((prev) =>
            prev.map((item) => {
              if (item.id !== next.id) {
                return item;
              }
              if (!outcome.ok) {
                return { ...item, status: "failed", message: outcome.message };
              }
              const { result } = outcome;
              if (result.status === "cancelled") {
                return { ...item, status: "cancelled" };
              }
              const landed = result.items.filter(
                (entry) => entry.status === "success",
              );
              const failed = result.items.filter(
                (entry) => entry.status === "failed",
              );
              if (landed.length > 0) {
                landedRef.current += landed.length;
                return {
                  ...item,
                  status: failed.length > 0 ? "partial" : "done",
                  title:
                    landed.length === 1
                      ? fileNameFromPath(landed[0].filePath)
                      : `${landed.length} videos from this link`,
                  message: failed[0]?.message,
                };
              }
              return { ...item, status: "failed", message: result.message };
            }),
          );
          setActiveProgress(null);
        }
      } finally {
        setActiveProgress(null);
        runningRef.current = false;
      }
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
      item.status === "partial" ||
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
    currentIndex: finishedCount + activeCount,
    total: items.length,
  };
}
