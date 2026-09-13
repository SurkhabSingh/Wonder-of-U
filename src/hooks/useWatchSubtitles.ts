import { useCallback, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { errorMessage } from "../lib/errors";
import { parseSubtitles } from "../lib/subtitles";
import { mergeSegmentAt, splitSegmentAt } from "../lib/segments";
import type { RecordingSegment } from "../types";

type SubtitleTrack = {
  index: number;
  language: string | null;
  title: string | null;
  codec: string | null;
};

type SubtitleSource = {
  content: string;
  name: string;
  tracks: SubtitleTrack[];
};

// Every subtitle line for the video being watched.
export function useWatchSubtitles() {
  const [cues, setCues] = useState<RecordingSegment[]>([]);
  const [tracks, setTracks] = useState<SubtitleTrack[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const load = useCallback(
    async (
      videoPath: string,
      subtitlePath: string | null,
      trackIndex: number | null,
    ) => {
      setIsLoading(true);
      setError(null);
      try {
        const source = await invoke<SubtitleSource>("load_watch_subtitles", {
          videoPath,
          subtitlePath,
          trackIndex,
        });
        if (!mountedRef.current) {
          return;
        }
        setTracks(source.tracks);
        const parsed = parseSubtitles(source.content, source.name);
        setCues(parsed);
        if (parsed.length === 0 && source.content.trim().length > 0) {
          setError(
            `${source.name} has no readable subtitle lines — it may be an unsupported format or a text encoding the parser could not decode.`,
          );
        }
      } catch (caught) {
        if (mountedRef.current) {
          setCues([]);
          setError(
            errorMessage(caught, "Those subtitles could not be read."),
          );
        }
      } finally {
        if (mountedRef.current) {
          setIsLoading(false);
        }
      }
    },
    [],
  );

  const clear = useCallback(() => {
    setCues([]);
    setTracks([]);
    setError(null);
  }, []);

  // Merge/split edit this in-session copy only. Nothing is written back to the subtitle
  // file — the user is reshaping what gets mined, not correcting the subtitles.
  const merge = useCallback((index: number, joiner: string) => {
    setCues((current) => mergeSegmentAt(current, index, joiner));
  }, []);

  const split = useCallback((index: number) => {
    setCues((current) => splitSegmentAt(current, index));
  }, []);

  return { cues, tracks, isLoading, error, load, clear, merge, split, mountedRef };
}
