import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { RecentRecording } from "../types";

// The half-open time window of a per-sentence playback. `null` when nothing is
// segment-bound so a row knows whether to draw the active highlight.
export type ActiveSegment = {
  startMs: number;
  endMs: number;
};

export type AudioPlayerState = {
  // `filePath === null` means nothing is loaded; callers use this to decide
  // whether to render the now-playing bar at all.
  filePath: string | null;
  fileName: string;
  isPlaying: boolean;
  currentTimeMs: number;
  durationMs: number;
  // The segment currently playing under a boundary, or null for free playback.
  activeSegment: ActiveSegment | null;
};

const INITIAL_STATE: AudioPlayerState = {
  filePath: null,
  fileName: "",
  isPlaying: false,
  currentTimeMs: 0,
  durationMs: 0,
  activeSegment: null,
};

export type AudioPlayer = AudioPlayerState & {
  playRecording: (recording: RecentRecording) => void;
  playSegment: (recording: RecentRecording, startMs: number, endMs: number) => void;
  toggle: () => void;
  pause: () => void;
  seekMs: (ms: number) => void;
  stop: () => void;
};

export function useAudioPlayer(): AudioPlayer {
  // One HTMLAudioElement for the lifetime of the hook — every recording plays
  // through the same element so starting a new track replaces the old one.
  const audioRef = useRef<HTMLAudioElement | null>(null);
  // The end of the segment window, in ms. When set, timeupdate pauses playback
  // as soon as it is crossed. Cleared on any free-playback action so a scrub or
  // a plain play/pause detaches from the segment.
  const boundaryMsRef = useRef<number | null>(null);
  // A seek requested before the freshly-set src had metadata. The browser drops
  // currentTime writes on an unloaded element, so we replay the seek once
  // loadedmetadata fires.
  const pendingSeekMsRef = useRef<number | null>(null);
  // The path currently attached to the element, so playSegment can decide
  // whether it needs to reload the source or can seek the loaded track.
  const loadedPathRef = useRef<string | null>(null);
  const [state, setState] = useState<AudioPlayerState>(INITIAL_STATE);

  useEffect(() => {
    const audio = new Audio();
    audioRef.current = audio;

    const handleLoadedMetadata = () => {
      const seconds = audio.duration;
      if (Number.isFinite(seconds) && seconds > 0) {
        setState((prev) => ({ ...prev, durationMs: Math.round(seconds * 1000) }));
      }
      const pending = pendingSeekMsRef.current;
      if (pending !== null) {
        pendingSeekMsRef.current = null;
        audio.currentTime = Math.max(0, pending / 1000);
      }
    };
    const handleTimeUpdate = () => {
      const boundaryMs = boundaryMsRef.current;
      if (boundaryMs !== null && audio.currentTime * 1000 >= boundaryMs) {
        // Reached the end of the segment: stop exactly here and drop the
        // boundary + highlight so the next timeupdate is ordinary playback.
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
    // Never load audio for a recording whose local file has been removed.
    if (recording.audioDeleted) {
      return;
    }
    const audio = audioRef.current;
    if (!audio) {
      return;
    }
    // A plain play detaches from any segment boundary in effect.
    boundaryMsRef.current = null;
    pendingSeekMsRef.current = null;
    loadedPathRef.current = recording.filePath;
    audio.src = convertFileSrc(recording.filePath);
    audio.currentTime = 0;
    // Seed the total from the known recording duration; loadedmetadata refines
    // it once the file's real duration is available.
    setState({
      filePath: recording.filePath,
      fileName: recording.fileName,
      isPlaying: false,
      currentTimeMs: 0,
      durationMs: recording.durationMs,
      activeSegment: null,
    });
    void audio.play().catch(() => {
      setState((prev) => ({ ...prev, isPlaying: false }));
    });
  }, []);

  const playSegment = useCallback(
    (recording: RecentRecording, startMs: number, endMs: number) => {
      // Never load audio for a recording whose local file has been removed.
      if (recording.audioDeleted) {
        return;
      }
      const audio = audioRef.current;
      if (!audio) {
        return;
      }
      const startSeconds = Math.max(0, startMs / 1000);
      boundaryMsRef.current = endMs;

      if (loadedPathRef.current !== recording.filePath || !audio.src) {
        // Fresh source: the seek can't land until metadata is known, so defer
        // it to loadedmetadata.
        loadedPathRef.current = recording.filePath;
        pendingSeekMsRef.current = startMs;
        audio.src = convertFileSrc(recording.filePath);
        setState({
          filePath: recording.filePath,
          fileName: recording.fileName,
          isPlaying: false,
          currentTimeMs: startMs,
          durationMs: recording.durationMs,
          activeSegment: { startMs, endMs },
        });
      } else {
        // Same track already loaded — seek immediately.
        pendingSeekMsRef.current = null;
        audio.currentTime = startSeconds;
        setState((prev) => ({
          ...prev,
          currentTimeMs: startMs,
          activeSegment: { startMs, endMs },
        }));
      }

      void audio.play().catch(() => {
        boundaryMsRef.current = null;
        setState((prev) => ({ ...prev, isPlaying: false, activeSegment: null }));
      });
    },
    [],
  );

  const toggle = useCallback(() => {
    const audio = audioRef.current;
    if (!audio || !audio.src) {
      return;
    }
    // The transport toggle is free playback: pressing it drops any segment
    // boundary and its highlight so audio runs to the end from here.
    boundaryMsRef.current = null;
    setState((prev) =>
      prev.activeSegment === null ? prev : { ...prev, activeSegment: null },
    );
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
    const seconds = Math.max(0, ms / 1000);
    audio.currentTime = seconds;
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
    loadedPathRef.current = null;
    setState(INITIAL_STATE);
  }, []);

  return {
    ...state,
    playRecording,
    playSegment,
    toggle,
    pause,
    seekMs,
    stop,
  };
}
