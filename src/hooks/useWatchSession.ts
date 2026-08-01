import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { errorMessage } from "../lib/errors";
import type { RecordingBatchResult, WatchSnapshot } from "../types";

const DISCONNECTED: WatchSnapshot = {
  connected: false,
  path: null,
  title: null,
  positionMs: null,
  durationMs: null,
  paused: false,
  subtitleText: null,
  subtitleStartMs: null,
  subtitleEndMs: null,
  subtitleDelayMs: 0,
};

// How often the watch page asks mpv where it is. The IPC round trip measured 0.3–0.5ms
// while playing, so this is chosen for how often the UI needs to look right rather than
// for cost: 250ms keeps the position and the current subtitle line feeling live without
// re-rendering the page 60 times a second.
const POLL_INTERVAL_MS = 250;

// Drives an external mpv: start it on a video, watch what it is showing, stop it.
//
// Polling only runs while a session is live, so a user who never opens the watch page
// pays nothing. mpv being closed by the user is not an error — the snapshot simply comes
// back disconnected and the poll winds down.
export function useWatchSession() {
  const [snapshot, setSnapshot] = useState<WatchSnapshot>(DISCONNECTED);
  // The video being opened, not merely "an open is in flight". A bare boolean was read once
  // per row by the video library, so opening one video put every row into a disabled
  // "Opening…" state at the same time.
  const [startingPath, setStartingPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isMining, setIsMining] = useState(false);
  const [mineResult, setMineResult] = useState<{ ok: boolean; message: string } | null>(
    null,
  );

  // The mine result is a notice, not a status: it reports what one keypress did and then
  // has nothing left to say. Left on screen it reads as a live condition — "This sentence
  // is already mined" sitting under a video the user has long since moved on from looks
  // like a warning about the line playing now. Failures get longer, because they are worth
  // reading and a user who missed one has no other way to find out what went wrong.
  useEffect(() => {
    if (!mineResult) {
      return;
    }
    const timer = window.setTimeout(
      () => {
        if (mountedRef.current) {
          setMineResult(null);
        }
      },
      mineResult.ok ? 4000 : 8000,
    );
    return () => window.clearTimeout(timer);
  }, [mineResult]);
  const mountedRef = useRef(true);
  // Read by the interval without re-subscribing it on every snapshot change.
  const connectedRef = useRef(false);
  connectedRef.current = snapshot.connected;

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const refresh = useCallback(async () => {
    try {
      const next = await invoke<WatchSnapshot>("watch_snapshot");
      if (mountedRef.current) {
        setSnapshot(next);
      }
    } catch (caught) {
      // A read failure means the player is gone, which is an ordinary end to a session.
      if (mountedRef.current) {
        setSnapshot(DISCONNECTED);
        if (import.meta.env.DEV) {
          console.debug("watch_snapshot failed:", caught);
        }
      }
    }
  }, []);

  useEffect(() => {
    if (!snapshot.connected) {
      return;
    }
    const interval = window.setInterval(() => void refresh(), POLL_INTERVAL_MS);
    return () => window.clearInterval(interval);
  }, [snapshot.connected, refresh]);

  const start = useCallback(
    async (videoPath: string, subtitlePath: string | null) => {
      setStartingPath(videoPath);
      setError(null);
      try {
        const next = await invoke<WatchSnapshot>("start_watch_session", {
          videoPath,
          subtitlePath,
        });
        if (mountedRef.current) {
          setSnapshot(next);
        }
      } catch (caught) {
        if (mountedRef.current) {
          setSnapshot(DISCONNECTED);
          setError(
            errorMessage(caught, "The video could not be opened in mpv."),
          );
        }
      } finally {
        if (mountedRef.current) {
          setStartingPath(null);
        }
      }
    },
    [],
  );

  // Mining reads the player fresh in Rust rather than sending the line the UI happens to
  // be showing: the user acts on what they are hearing, and this poll is up to 250ms
  // behind. Sending the stale line would make a card for the previous sentence.
  const mine = useCallback(async () => {
    setIsMining(true);
    try {
      const result = await invoke<RecordingBatchResult>("mine_watched_line");
      const item = result.items[0];
      const ok = item?.status === "success";
      if (mountedRef.current) {
        setMineResult({
          ok,
          message: item?.message ?? result.message,
        });
      }
    } catch (caught) {
      if (mountedRef.current) {
        setMineResult({
          ok: false,
          message: errorMessage(caught, "That line could not be mined."),
        });
      }
    } finally {
      if (mountedRef.current) {
        setIsMining(false);
      }
    }
  }, []);

  // Mines a specific line rather than whatever is on screen. Separate from `mine` on
  // purpose: that one re-reads mpv so the hotkey captures what you are hearing, while
  // this one is told its bounds so it can mine a row you scrolled back to, or one you
  // merged out of two cues.
  const mineLine = useCallback(
    async (
      videoPath: string,
      text: string,
      startMs: number,
      endMs: number,
      padBeforeMs: number | null,
      padAfterMs: number | null,
    ) => {
      setIsMining(true);
      try {
        const result = await invoke<RecordingBatchResult>("mine_watch_line_at", {
          videoPath,
          text,
          startMs,
          endMs,
          padBeforeMs,
          padAfterMs,
        });
        const item = result.items[0];
        const ok = item?.status === "success";
        if (mountedRef.current) {
          setMineResult({ ok, message: item?.message ?? result.message });
        }
        return ok;
      } catch (caught) {
        if (mountedRef.current) {
          setMineResult({
            ok: false,
            message: errorMessage(caught, "That line could not be mined."),
          });
        }
        return false;
      } finally {
        if (mountedRef.current) {
          setIsMining(false);
        }
      }
    },
    [],
  );

  const seek = useCallback(async (positionMs: number) => {
    try {
      await invoke("seek_watch_session", { positionMs });
    } catch {
      // The player is gone; the next poll reports it.
    }
  }, []);

  const stop = useCallback(async () => {
    try {
      await invoke("stop_watch_session");
    } catch {
      // Already gone is the outcome we wanted.
    }
    if (mountedRef.current) {
      setSnapshot(DISCONNECTED);
      setError(null);
    }
  }, []);

  // Subtitle offset. mpv owns the value, so the UI never keeps its own copy — it reads
  // `snapshot.subtitleDelayMs` and asks for a new absolute value, which keeps the two from
  // disagreeing after a seek or a file change.
  const setSubtitleDelay = useCallback(async (delayMs: number) => {
    try {
      await invoke("set_watch_subtitle_delay", { delayMs });
      await refresh();
    } catch (caught) {
      if (mountedRef.current) {
        setError(errorMessage(caught, "The subtitle offset could not be changed."));
      }
    }
  }, [refresh]);

  return {
    snapshot,
    setSubtitleDelay,
    startingPath,
    // Kept so every existing consumer of "is an open in flight" reads the same as before.
    isStarting: startingPath !== null,
    error,
    start,
    stop,
    refresh,
    mine,
    mineLine,
    seek,
    isMining,
    mineResult,
  };
}
