import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { errorMessage } from "../lib/errors";
import type {
  AppBootstrap,
  RecordingBatchResult,
  TranscriptionQueueItem,
} from "../types";

type TranscriptionEnqueueInput = {
  filePath: string;
  title?: string;
};

type UseTranscriptionQueueOptions = {
  // Applied after EACH item so the Library refreshes as transcripts land — the
  // same bootstrap the single-file backend command returns. Threaded in rather
  // than reached through the old blocking action, mirroring how
  // `useRecordingActions.transcribeRecordings` called `applyBootstrap`.
  applyBootstrap: (nextBootstrap: AppBootstrap) => void;
  // Flush any pending settings edits before the invoke so a just-changed
  // language / model / CPU-usage value is on disk when whisper-cli reads it.
  persistSettingsIfNeeded: () => Promise<void>;
};

// The backend transcribe command is single-file and single-flight on the one
// whisper-cli slot, so this is a strictly sequential queue on top of it: only
// ever one `active` item, the rest wait as `queued`. It mirrors useYoutubeQueue.
//
// Completion is the awaited `invoke` resolving — nothing else. A user Cancel
// arrives as a resolved "cancelled" item. Progress is a lightweight
// `transcription-progress` percent event, and cancel is a `transcription-cancel`
// event that kills the running whisper-cli so the active file returns cancelled
// and the loop moves on. A failed/cancelled item never stops the queue.

export function useTranscriptionQueue({
  applyBootstrap,
  persistSettingsIfNeeded,
}: UseTranscriptionQueueOptions) {
  const [items, setItems] = useState<TranscriptionQueueItem[]>([]);
  // Percent for the single active file, or null when nothing is active.
  // Single-flight, so this always belongs to the one `active` item.
  const [activeProgress, setActiveProgress] = useState<number | null>(null);

  // Keep the injected callbacks in refs so the processor never captures a stale
  // closure and the effects don't re-fire because a parent re-rendered.
  const applyBootstrapRef = useRef(applyBootstrap);
  const persistSettingsIfNeededRef = useRef(persistSettingsIfNeeded);
  applyBootstrapRef.current = applyBootstrap;
  persistSettingsIfNeededRef.current = persistSettingsIfNeeded;

  // Mirror of `items` the async loop reads between iterations without capturing
  // a stale render closure. Kept in sync by the effect below.
  const itemsRef = useRef<TranscriptionQueueItem[]>(items);
  // `force` per item — kept off the item shape (which is UI state) in a side map
  // keyed by id, since a re-transcribe enqueues force:true while a first-time
  // transcribe enqueues force:false and the two can interleave in one queue.
  const forceByIdRef = useRef<Map<string, boolean>>(new Map());
  // Guards: `runningRef` keeps the processor from running twice; `mountedRef`
  // stops setState after unmount.
  const runningRef = useRef(false);
  const mountedRef = useRef(true);
  // Stable id source — no crypto dependency, monotonic per session.
  const idRef = useRef(0);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // Keep the ref in lock-step with state so the loop's next-item lookup always
  // reads the latest queue.
  useEffect(() => {
    itemsRef.current = items;
  }, [items]);

  // Progress is a fire-and-forget percent event during the active file.
  useEffect(() => {
    const unlisten = listen<number>("transcription-progress", ({ payload }) => {
      if (!mountedRef.current) {
        return;
      }
      if (typeof payload === "number") {
        setActiveProgress(Math.max(0, Math.min(100, payload)));
      }
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  const enqueue = useCallback(
    (files: TranscriptionEnqueueInput[], force: boolean) => {
      if (files.length === 0) {
        return;
      }
      setItems((prev) => {
        // Dedupe against still-pending/active items and within this call, by
        // file path — re-adding a file already waiting is a no-op.
        const seen = new Set(
          prev
            .filter(
              (item) => item.status === "queued" || item.status === "active",
            )
            .map((item) => item.filePath),
        );
        const additions: TranscriptionQueueItem[] = [];
        for (const file of files) {
          if (seen.has(file.filePath)) {
            continue;
          }
          seen.add(file.filePath);
          idRef.current += 1;
          const id = `tx-${idRef.current}`;
          forceByIdRef.current.set(id, force);
          additions.push({
            id,
            filePath: file.filePath,
            title: file.title,
            status: "queued",
          });
        }
        if (additions.length === 0) {
          return prev;
        }
        return [...prev, ...additions];
      });
    },
    [],
  );

  const remove = useCallback((id: string) => {
    // Only a still-queued row can be dropped; active/terminal rows are a no-op.
    setItems((prev) =>
      prev.filter((item) => {
        const droppable = item.id === id && item.status === "queued";
        if (droppable) {
          forceByIdRef.current.delete(id);
        }
        return !droppable;
      }),
    );
  }, []);

  const cancelActive = useCallback(() => {
    // Kill the active whisper-cli. The active item's invoke then resolves with a
    // "cancelled" item, and the loop advances to the next queued file.
    void emit("transcription-cancel");
  }, []);

  const clearFinished = useCallback(() => {
    setItems((prev) =>
      prev.filter(
        (item) => item.status === "queued" || item.status === "active",
      ),
    );
  }, []);

  // The sequential processor — a plain loop guarded by `runningRef` so it never
  // runs twice. Each iteration promotes the next queued item, awaits the
  // single-file invoke to completion, and stamps the terminal status from the
  // RESOLVED result. Both the success and the error path advance, so one failed
  // or cancelled item never blocks the rest of the queue.
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

        const force = forceByIdRef.current.get(next.id) ?? false;

        setItems((prev) =>
          prev.map((item) =>
            item.id === next.id ? { ...item, status: "active" } : item,
          ),
        );
        setActiveProgress(0);

        // Completion = this awaited invoke resolving. A rejection (whisper-cli
        // missing, spawn failure) is caught and marks the item failed; a user
        // Cancel comes back as a resolved "cancelled" item.
        let result: RecordingBatchResult | null = null;
        let failureMessage = "The recording could not be transcribed.";
        try {
          await persistSettingsIfNeededRef.current();
          result = await invoke<RecordingBatchResult>("transcribe_recordings", {
            filePaths: [next.filePath],
            force,
          });
        } catch (error) {
          failureMessage = errorMessage(error, failureMessage);
        }

        if (!mountedRef.current) {
          break;
        }

        // Refresh the Library from the returned bootstrap so a landed transcript
        // shows up immediately, before the queue even drains.
        if (result) {
          applyBootstrapRef.current(result.bootstrap);
        }
        forceByIdRef.current.delete(next.id);

        setItems((prev) =>
          prev.map((item) => {
            if (item.id !== next.id) {
              return item;
            }
            if (!result) {
              return { ...item, status: "failed", message: failureMessage };
            }
            const outcome = result.items[0];
            // A result with no item is a real failure — keep the batch message.
            if (!outcome) {
              return { ...item, status: "failed", message: result.message };
            }
            if (outcome.status === "success") {
              return { ...item, status: "done" };
            }
            // A user Cancel comes back as a "cancelled" item — read it so a
            // deliberate cancel does not render as "Failed".
            if (outcome.status === "cancelled") {
              return { ...item, status: "cancelled" };
            }
            // "failed" | "skipped" | anything else — a row that did not land a
            // transcript, kept with the backend's own reason.
            return { ...item, status: "failed", message: outcome.message };
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
    // 1-based position of the file being transcribed, and the run total. Drives a
    // "Transcribing N of M…" line: N = finished + active, M = items.length.
    currentIndex: finishedCount + activeCount,
    total: items.length,
  };
}
