import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { RecentRecording } from "../types";

export type AudioPlayerState = {
  // `filePath === null` means nothing is loaded; callers use this to decide
  // whether to render the now-playing bar at all.
  filePath: string | null;
  fileName: string;
  isPlaying: boolean;
  currentTimeMs: number;
  durationMs: number;
};

const INITIAL_STATE: AudioPlayerState = {
  filePath: null,
  fileName: "",
  isPlaying: false,
  currentTimeMs: 0,
  durationMs: 0,
};

export type AudioPlayer = AudioPlayerState & {
  playRecording: (recording: RecentRecording) => void;
  toggle: () => void;
  pause: () => void;
  seekMs: (ms: number) => void;
  stop: () => void;
};

export function useAudioPlayer(): AudioPlayer {
  // One HTMLAudioElement for the lifetime of the hook — every recording plays
  // through the same element so starting a new track replaces the old one.
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const [state, setState] = useState<AudioPlayerState>(INITIAL_STATE);

  useEffect(() => {
    const audio = new Audio();
    audioRef.current = audio;

    const handleLoadedMetadata = () => {
      const seconds = audio.duration;
      if (Number.isFinite(seconds) && seconds > 0) {
        setState((prev) => ({ ...prev, durationMs: Math.round(seconds * 1000) }));
      }
    };
    const handleTimeUpdate = () => {
      setState((prev) => ({
        ...prev,
        currentTimeMs: Math.round(audio.currentTime * 1000),
      }));
    };
    const handleEnded = () => {
      audio.currentTime = 0;
      setState((prev) => ({ ...prev, isPlaying: false, currentTimeMs: 0 }));
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
    });
    void audio.play().catch(() => {
      setState((prev) => ({ ...prev, isPlaying: false }));
    });
  }, []);

  const toggle = useCallback(() => {
    const audio = audioRef.current;
    if (!audio || !audio.src) {
      return;
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
    const seconds = Math.max(0, ms / 1000);
    audio.currentTime = seconds;
    setState((prev) => ({ ...prev, currentTimeMs: Math.round(seconds * 1000) }));
  }, []);

  const stop = useCallback(() => {
    const audio = audioRef.current;
    if (audio) {
      audio.pause();
      audio.removeAttribute("src");
      audio.load();
    }
    setState(INITIAL_STATE);
  }, []);

  return {
    ...state,
    playRecording,
    toggle,
    pause,
    seekMs,
    stop,
  };
}
