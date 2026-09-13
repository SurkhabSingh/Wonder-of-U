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

// How far playback must MOVE before the resume point is written again.
const SAVE_EVERY_MS = 30_000;

// Drives an external mpv: start it on a video, watch what it is showing, stop it.
export function useWatchSession() {
  const [snapshot, setSnapshot] = useState<WatchSnapshot>(DISCONNECTED);
  const [startingPath, setStartingPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isMining, setIsMining] = useState(false);
  const [mineResult, setMineResult] = useState<{
    ok: boolean;
    message: string;
  } | null>(null);

  // The mine result is a notice, not a status: it reports what one keypress did and then
  // has nothing left to say.
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
  const connectedRef = useRef(false);
  connectedRef.current = snapshot.connected;

  // The video this session was STARTED on, which is the key the resume point is stored under.
  const playingPathRef = useRef<string | null>(null);
  // The last position actually written, so the next write can be spaced from it.
  const savedPositionRef = useRef<number | null>(null);
  // The last position seen while connected. mpv reports nothing once it is gone, so the final
  // save — the one that catches closing the window mid-episode — has to use what we last saw.
  const seenPositionRef = useRef<number | null>(null);
  const seenDurationRef = useRef<number>(0);

  // Fire-and-forget: this runs on a poll tick and when a session ends, and a failure to
  // remember a position is not something to interrupt someone's viewing over. It is also
  // deliberately not awaited by the poll, so a slow disk cannot stall the interval.
  const savePosition = useCallback((positionMs: number, durationMs: number) => {
    const videoPath = playingPathRef.current;
    if (!videoPath) {
      return;
    }
    savedPositionRef.current = positionMs;
    void invoke("set_watched_video_position", {
      videoPath,
      positionMs: Math.round(positionMs),
      durationMs: Math.round(durationMs),
    }).catch((caught) => {
      if (import.meta.env.DEV) {
        console.debug("set_watched_video_position failed:", caught);
      }
    });
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // Persist where we got to, then stop treating the session as live.
  const endSession = useCallback(() => {
    const position = seenPositionRef.current;
    if (position !== null) {
      savePosition(position, seenDurationRef.current);
    }
    playingPathRef.current = null;
    seenPositionRef.current = null;
    savedPositionRef.current = null;
    seenDurationRef.current = 0;
  }, [savePosition]);

  const refresh = useCallback(async () => {
    try {
      const next = await invoke<WatchSnapshot>("watch_snapshot");
      if (next.connected && next.positionMs !== null) {
        seenPositionRef.current = next.positionMs;
        seenDurationRef.current = next.durationMs ?? seenDurationRef.current;
        const saved = savedPositionRef.current;
        if (
          saved === null ||
          Math.abs(next.positionMs - saved) >= SAVE_EVERY_MS
        ) {
          savePosition(next.positionMs, seenDurationRef.current);
        }
      } else if (!next.connected) {
        endSession();
      }
      if (mountedRef.current) {
        setSnapshot(next);
      }
    } catch (caught) {
      // A read failure means the player is gone, which is an ordinary end to a session.
      endSession();
      if (mountedRef.current) {
        setSnapshot(DISCONNECTED);
        if (import.meta.env.DEV) {
          console.debug("watch_snapshot failed:", caught);
        }
      }
    }
  }, [endSession, savePosition]);

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
      endSession();
      try {
        const next = await invoke<WatchSnapshot>("start_watch_session", {
          videoPath,
          subtitlePath,
        });
        playingPathRef.current = videoPath;
        savedPositionRef.current = next.positionMs;
        seenPositionRef.current = next.positionMs;
        seenDurationRef.current = next.durationMs ?? 0;
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
        const result = await invoke<RecordingBatchResult>(
          "mine_watch_line_at",
          {
            videoPath,
            text,
            startMs,
            endMs,
            padBeforeMs,
            padAfterMs,
          },
        );
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
    // Before the player goes away, not after: this sets the snapshot to DISCONNECTED itself
    // rather than letting the poll notice, so it is the only path that would otherwise skip
    // the final save and lose the position of anyone who stops from inside the app.
    endSession();
    try {
      await invoke("stop_watch_session");
    } catch {
      // Already gone is the outcome we wanted.
    }
    if (mountedRef.current) {
      setSnapshot(DISCONNECTED);
      setError(null);
    }
  }, [endSession]);

  // Subtitle offset. mpv owns the value, so the UI never keeps its own copy — it reads
  // `snapshot.subtitleDelayMs` and asks for a new absolute value, which keeps the two from
  // disagreeing after a seek or a file change.
  const setSubtitleDelay = useCallback(
    async (delayMs: number) => {
      try {
        await invoke("set_watch_subtitle_delay", { delayMs });
        await refresh();
      } catch (caught) {
        if (mountedRef.current) {
          setError(
            errorMessage(caught, "The subtitle offset could not be changed."),
          );
        }
      }
    },
    [refresh],
  );

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
