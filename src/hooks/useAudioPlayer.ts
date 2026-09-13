import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { RecentRecording } from "../types";

// The half-open time window of a per-sentence playback. `null` when nothing is
// segment-bound so a row knows whether to draw the active highlight.
export type ActiveSegment = {
  startMs: number;
  endMs: number;
};

export type AudioPlayerState = {
  filePath: string | null;
  fileName: string;
  isPlaying: boolean;
  currentTimeMs: number;
  durationMs: number;
  activeSegment: ActiveSegment | null;
  playbackRate: number;
  isRepeating: boolean;
};

const INITIAL_STATE: AudioPlayerState = {
  filePath: null,
  fileName: "",
  isPlaying: false,
  currentTimeMs: 0,
  durationMs: 0,
  activeSegment: null,
  playbackRate: 1,
  isRepeating: false,
};

export type AudioPlayer = AudioPlayerState & {
  playRecording: (recording: RecentRecording) => void;
  playSegment: (
    recording: RecentRecording,
    startMs: number,
    endMs: number,
    paddingMs?: number,
    onError?: (message: string) => void,
  ) => void;
  toggle: () => void;
  pause: () => void;
  seekMs: (ms: number) => void;
  stop: () => void;
  setPlaybackRate: (rate: number) => void;
  toggleRepeat: () => void;
};

export function useAudioPlayer(): AudioPlayer {
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const boundaryMsRef = useRef<number | null>(null);
  const pendingSeekMsRef = useRef<number | null>(null);
  const rateRef = useRef(1);
  const repeatRef = useRef(false);
  const segmentStartMsRef = useRef<number | null>(null);
  const clipOffsetMsRef = useRef<number | null>(null);
  const segmentRequestRef = useRef(0);
  const recordingRef = useRef<RecentRecording | null>(null);
  const [state, setState] = useState<AudioPlayerState>(INITIAL_STATE);

  useEffect(() => {
    const audio = new Audio();
    audioRef.current = audio;

    const handleLoadedMetadata = () => {
      audio.playbackRate = rateRef.current;
      const seconds = audio.duration;
      if (
        clipOffsetMsRef.current === null &&
        Number.isFinite(seconds) &&
        seconds > 0
      ) {
        setState((prev) => ({
          ...prev,
          durationMs: Math.round(seconds * 1000),
        }));
      }
      const pending = pendingSeekMsRef.current;
      if (pending !== null) {
        pendingSeekMsRef.current = null;
        audio.currentTime = Math.max(0, pending / 1000);
      }
    };
    // The element's clock is clip-relative while a sentence clip is loaded; everything the
    // UI shows is on the recording's timeline, so put it back.
    const positionMs = () =>
      Math.round(audio.currentTime * 1000) + (clipOffsetMsRef.current ?? 0);

    const handleTimeUpdate = () => {
      if (clipOffsetMsRef.current !== null) {
        setState((prev) => ({ ...prev, currentTimeMs: positionMs() }));
        return;
      }
      const boundaryMs = boundaryMsRef.current;
      if (boundaryMs !== null && audio.currentTime * 1000 >= boundaryMs) {
        const repeatStart =
          repeatRef.current &&
          segmentStartMsRef.current !== null &&
          segmentStartMsRef.current < boundaryMs
            ? segmentStartMsRef.current
            : null;
        if (repeatStart !== null) {
          audio.currentTime = Math.max(0, repeatStart / 1000);
          setState((prev) => ({ ...prev, currentTimeMs: repeatStart }));
          return;
        }
        boundaryMsRef.current = null;
        audio.pause();
        setState((prev) => ({
          ...prev,
          currentTimeMs: Math.round(audio.currentTime * 1000),
          activeSegment: null,
        }));
        return;
      }
      setState((prev) => ({
        ...prev,
        currentTimeMs: Math.round(audio.currentTime * 1000),
      }));
    };
    const handleEnded = () => {
      // A sentence clip always ends here, since it holds nothing but the sentence.
      if (clipOffsetMsRef.current !== null) {
        if (repeatRef.current) {
          audio.currentTime = 0;
          void audio.play().catch(() => {
            setState((prev) => ({ ...prev, isPlaying: false }));
          });
          setState((prev) => ({ ...prev, currentTimeMs: positionMs() }));
          return;
        }
        audio.currentTime = 0;
        setState((prev) => ({
          ...prev,
          isPlaying: false,
          currentTimeMs: clipOffsetMsRef.current ?? 0,
          activeSegment: null,
        }));
        return;
      }
      // A segment whose end sits at the very end of the file finishes via `ended`
      // rather than the timeupdate boundary; honour repeat here too so the last
      // sentence loops like any other. The `start < end` guard also stops a
      // degenerate segment from re-seeking forever.
      const boundaryMs = boundaryMsRef.current;
      const repeatStart =
        repeatRef.current &&
        boundaryMs !== null &&
        segmentStartMsRef.current !== null &&
        segmentStartMsRef.current < boundaryMs
          ? segmentStartMsRef.current
          : null;
      if (repeatStart !== null) {
        audio.currentTime = Math.max(0, repeatStart / 1000);
        void audio.play().catch(() => {
          setState((prev) => ({ ...prev, isPlaying: false }));
        });
        setState((prev) => ({ ...prev, currentTimeMs: repeatStart }));
        return;
      }
      boundaryMsRef.current = null;
      audio.currentTime = 0;
      setState((prev) => ({
        ...prev,
        isPlaying: false,
        currentTimeMs: 0,
        activeSegment: null,
      }));
    };
    const handlePlay = () => {
      setState((prev) => ({ ...prev, isPlaying: true }));
    };
    const handlePause = () => {
      setState((prev) => ({ ...prev, isPlaying: false }));
    };

    audio.addEventListener("loadedmetadata", handleLoadedMetadata);
    audio.addEventListener("timeupdate", handleTimeUpdate);
    audio.addEventListener("ended", handleEnded);
    audio.addEventListener("play", handlePlay);
    audio.addEventListener("pause", handlePause);

    return () => {
      audio.removeEventListener("loadedmetadata", handleLoadedMetadata);
      audio.removeEventListener("timeupdate", handleTimeUpdate);
      audio.removeEventListener("ended", handleEnded);
      audio.removeEventListener("play", handlePlay);
      audio.removeEventListener("pause", handlePause);
      audio.pause();
      audio.removeAttribute("src");
      audio.load();
      audioRef.current = null;
    };
  }, []);

  const playRecording = useCallback((recording: RecentRecording) => {
    if (recording.audioDeleted) {
      return;
    }
    const audio = audioRef.current;
    if (!audio) {
      return;
    }
    boundaryMsRef.current = null;
    pendingSeekMsRef.current = null;
    clipOffsetMsRef.current = null;
    segmentRequestRef.current += 1;
    recordingRef.current = recording;
    audio.src = convertFileSrc(recording.filePath);
    audio.currentTime = 0;

    setState((prev) => ({
      ...prev,
      filePath: recording.filePath,
      fileName: recording.fileName,
      isPlaying: false,
      currentTimeMs: 0,
      durationMs: recording.durationMs,
      activeSegment: null,
    }));
    void audio.play().catch(() => {
      setState((prev) => ({ ...prev, isPlaying: false }));
    });
  }, []);

  const playSegment = useCallback(
    (
      recording: RecentRecording,
      startMs: number,
      endMs: number,
      paddingMs?: number,
      onError?: (message: string) => void,
    ) => {
      // Never load audio for a recording whose local file has been removed.
      if (recording.audioDeleted) {
        return;
      }
      const audio = audioRef.current;
      if (!audio) {
        return;
      }

      const padding = Math.max(0, paddingMs ?? 0);
      const clipStartMs = Math.max(0, startMs - padding);
      const request = ++segmentRequestRef.current;
      recordingRef.current = recording;

      setState((prev) => ({
        ...prev,
        filePath: recording.filePath,
        fileName: recording.fileName,
        currentTimeMs: startMs,
        durationMs: recording.durationMs,
        activeSegment: { startMs, endMs },
      }));

      void invoke<string>("preview_segment_clip", {
        filePath: recording.filePath,
        startMs,
        endMs,
      })
        .then((clipPath) => {
          // A newer click already went out: that one owns the element now.
          if (request !== segmentRequestRef.current) {
            return;
          }
          boundaryMsRef.current = null;
          pendingSeekMsRef.current = null;
          segmentStartMsRef.current = null;
          clipOffsetMsRef.current = clipStartMs;
          audio.src = convertFileSrc(clipPath);
          audio.currentTime = 0;
          void audio.play().catch(() => {
            setState((prev) => ({
              ...prev,
              isPlaying: false,
              activeSegment: null,
            }));
          });
        })
        .catch((error: unknown) => {
          if (request !== segmentRequestRef.current) {
            return;
          }
          // Deliberately no fall back to seeking the original file. That is the inaccurate
          // path this replaced, and silently using it would put back the very bug being
          // fixed while looking like it worked.
          setState((prev) => ({
            ...prev,
            isPlaying: false,
            activeSegment: null,
          }));
          onError?.(
            typeof error === "string"
              ? error
              : "This sentence could not be played.",
          );
        });
    },
    [],
  );

  const toggle = useCallback(() => {
    const audio = audioRef.current;
    if (!audio || !audio.src) {
      return;
    }
    // While a sentence clip is loaded the transport governs that sentence and nothing else —
    // the element holds only those few seconds. Dropping the highlight here would take it off
    // the very row being played, and the "runs to the end of the file" the next line assumes
    // is not available to reach.
    if (clipOffsetMsRef.current === null) {
      boundaryMsRef.current = null;
      setState((prev) =>
        prev.activeSegment === null ? prev : { ...prev, activeSegment: null },
      );
    }
    if (audio.paused) {
      void audio.play().catch(() => {
        setState((prev) => ({ ...prev, isPlaying: false }));
      });
    } else {
      audio.pause();
    }
  }, []);

  const pause = useCallback(() => {
    audioRef.current?.pause();
  }, []);

  const seekMs = useCallback((ms: number) => {
    const audio = audioRef.current;
    if (!audio) {
      return;
    }
    // A manual scrub leaves the segment window: clear the boundary so playback
    // no longer pauses at the old segment end, and drop the row highlight.
    boundaryMsRef.current = null;
    pendingSeekMsRef.current = null;
    segmentRequestRef.current += 1;
    const seconds = Math.max(0, ms / 1000);

    const recording = recordingRef.current;
    if (clipOffsetMsRef.current !== null && recording) {
      clipOffsetMsRef.current = null;
      pendingSeekMsRef.current = Math.max(0, ms);
      audio.src = convertFileSrc(recording.filePath);
    } else {
      audio.currentTime = seconds;
    }

    setState((prev) => ({
      ...prev,
      currentTimeMs: Math.round(seconds * 1000),
      activeSegment: null,
    }));
  }, []);

  const stop = useCallback(() => {
    const audio = audioRef.current;
    if (audio) {
      audio.pause();
      audio.removeAttribute("src");
      audio.load();
    }
    boundaryMsRef.current = null;
    pendingSeekMsRef.current = null;
    segmentStartMsRef.current = null;
    clipOffsetMsRef.current = null;
    segmentRequestRef.current += 1;
    recordingRef.current = null;
    repeatRef.current = false;
    rateRef.current = 1;
    setState(INITIAL_STATE);
  }, []);

  const setPlaybackRate = useCallback((rate: number) => {
    const clamped = Math.min(4, Math.max(0.25, rate));
    rateRef.current = clamped;
    const audio = audioRef.current;
    if (audio) {
      audio.playbackRate = clamped;
    }
    setState((prev) => ({ ...prev, playbackRate: clamped }));
  }, []);

  const toggleRepeat = useCallback(() => {
    const next = !repeatRef.current;
    repeatRef.current = next;
    setState((prev) => ({ ...prev, isRepeating: next }));
  }, []);

  return {
    ...state,
    playRecording,
    playSegment,
    toggle,
    pause,
    seekMs,
    stop,
    setPlaybackRate,
    toggleRepeat,
  };
}
