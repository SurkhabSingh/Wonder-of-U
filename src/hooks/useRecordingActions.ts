import { useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useConfirm } from "../components/ui/ConfirmDialogProvider";
import { errorMessage } from "../lib/errors";
import { formatBatchToastMessage } from "../lib/recordingBatchMessages";
import type {
  AppBootstrap,
  BusyAction,
  RecordingBatchResult,
  YoutubeImportOutcome,
} from "../types";

type UseRecordingActionsOptions = {
  applyBootstrap: (nextBootstrap: AppBootstrap) => void;
  persistSettingsIfNeeded: () => Promise<void>;
  setBusyAction: (busyAction: BusyAction) => void;
  setLoadError: (message: string) => void;
  setRecordingActionMessage: (message: string) => void;
  showSuccess: (message: string) => void;
  showWarning: (message: string) => void;
};

export function useRecordingActions({
  applyBootstrap,
  persistSettingsIfNeeded,
  setBusyAction,
  setLoadError,
  setRecordingActionMessage,
  showSuccess,
  showWarning,
}: UseRecordingActionsOptions) {
  const confirm = useConfirm();

  // A batch that could not run at all ("unavailable") or only partly ran
  // ("partial") is not a success. Reporting it with a green check is how
  // "the browser extension is not connected" ended up looking like good news.
  const notifyBatchResult = useCallback(
    (result: RecordingBatchResult, message: string) => {
      if (result.status === "unavailable" || result.status === "partial") {
        showWarning(message);
        return;
      }

      showSuccess(message);
    },
    [showSuccess, showWarning],
  );

  const playRecording = useCallback(
    async (filePath: string) => {
      try {
        setBusyAction("playRecording");
        await invoke("play_recording", { filePath });
      } catch (error) {
        setLoadError(errorMessage(error, "The audio file could not be played."));
      } finally {
        setBusyAction(null);
      }
    },
    [setBusyAction, setLoadError],
  );

  const deleteRecording = useCallback(
    async (filePath: string) => {
      const confirmed = await confirm({
        title: "Delete recording?",
        message:
          "Delete this saved recording from Wonder of U? This removes the local audio, transcript, and translation files from this machine. Existing Anki cards are not affected.",
        okLabel: "Delete",
        cancelLabel: "Keep",
        danger: true,
      });
      if (!confirmed) {
        return;
      }

      try {
        setBusyAction("deleteRecording");
        const nextBootstrap = await invoke<AppBootstrap>("delete_recording", {
          filePath,
        });
        applyBootstrap(nextBootstrap);
        setRecordingActionMessage("Recording deleted.");
        showSuccess("Recording deleted.");
      } catch (error) {
        setLoadError(errorMessage(error, "The recording could not be deleted."));
      } finally {
        setBusyAction(null);
      }
    },
    [applyBootstrap, setBusyAction, setLoadError, setRecordingActionMessage, showSuccess],
  );

  const deleteRecordings = useCallback(
    async (filePaths: string[]) => {
      if (filePaths.length === 0) {
        return;
      }

      const confirmed = await confirm({
        title: "Delete recordings?",
        message: `Delete ${filePaths.length} selected recording${
          filePaths.length === 1 ? "" : "s"
        } from Wonder of U? This removes local audio, transcript, and translation files from this machine. Existing Anki cards are not affected.`,
        okLabel: "Delete",
        cancelLabel: "Keep",
        danger: true,
      });
      if (!confirmed) {
        return;
      }

      try {
        setBusyAction("deleteRecording");
        const result = await invoke<RecordingBatchResult>("delete_recordings", {
          filePaths,
        });
        applyBootstrap(result.bootstrap);
        setRecordingActionMessage(result.message);
        showSuccess(result.message);
      } catch (error) {
        setLoadError(
          errorMessage(error, "The selected recordings could not be deleted."),
        );
      } finally {
        setBusyAction(null);
      }
    },
    [applyBootstrap, setBusyAction, setLoadError, setRecordingActionMessage, showSuccess],
  );

  const pushRecordingsToAnki = useCallback(
    async (filePaths: string[], deckName?: string) => {
      try {
        setBusyAction("pushAnki");
        await persistSettingsIfNeeded();
        const targetDeck = deckName?.trim();
        const result = await invoke<RecordingBatchResult>(
          targetDeck ? "push_recordings_to_anki_deck" : "push_recordings_to_anki",
          targetDeck ? { filePaths, deckName: targetDeck } : { filePaths },
        );
        applyBootstrap(result.bootstrap);
        const message = formatBatchToastMessage("anki", result);
        setRecordingActionMessage(message);
        if (
          result.status === "unavailable" ||
          result.status === "partial" ||
          message.toLowerCase().includes("anki is currently offline") ||
          message.toLowerCase().includes("no cards were pushed") ||
          message.toLowerCase().includes("furigana was skipped")
        ) {
          showWarning(message);
        } else {
          showSuccess(message);
        }
      } catch (error) {
        const message = errorMessage(
          error,
          "The recordings could not be pushed to Anki.",
        );
        if (message.toLowerCase().includes("anki")) {
          showWarning(message);
        }
        setLoadError(message);
      } finally {
        setBusyAction(null);
      }
    },
    [
      applyBootstrap,
      persistSettingsIfNeeded,
      setBusyAction,
      setLoadError,
      setRecordingActionMessage,
      showSuccess,
      showWarning,
    ],
  );

  // Sentence mining: creates ONE Anki card for a single sentence from the
  // reading view. Mirrors pushRecordingsToAnki (persist → invoke → applyBootstrap
  // → routed toast) but returns the batch result so the caller can mark the row
  // "mined" from the note id — mining does not mutate RecentRecording in v1.
  const mineSegment = useCallback(
    async (
      filePath: string,
      text: string,
      startMs: number,
      endMs: number,
      translation: string | null,
    ): Promise<RecordingBatchResult | null> => {
      try {
        setBusyAction("mineSegment");
        await persistSettingsIfNeeded();
        const result = await invoke<RecordingBatchResult>("mine_segment_to_anki", {
          filePath,
          text,
          startMs,
          endMs,
          translation,
        });
        applyBootstrap(result.bootstrap);
        const message = result.message;
        setRecordingActionMessage(message);
        notifyBatchResult(result, message);
        return result;
      } catch (error) {
        const message = errorMessage(
          error,
          "The sentence could not be mined to Anki.",
        );
        if (message.toLowerCase().includes("anki")) {
          showWarning(message);
        }
        setLoadError(message);
        return null;
      } finally {
        setBusyAction(null);
      }
    },
    [
      applyBootstrap,
      notifyBatchResult,
      persistSettingsIfNeeded,
      setBusyAction,
      setLoadError,
      setRecordingActionMessage,
      showWarning,
    ],
  );

  // Media import: acquire → normalize → register with NO transcript, so the file
  // lands in the Library as "Needs transcript" and every existing action works
  // on it unchanged. Returns the batch result so the caller can navigate to the
  // Library only when something actually landed.
  const importMedia = useCallback(
    async (paths: string[]): Promise<RecordingBatchResult | null> => {
      if (paths.length === 0) {
        return null;
      }

      try {
        setBusyAction("importMedia");
        await persistSettingsIfNeeded();
        const result = await invoke<RecordingBatchResult>("import_media", {
          paths,
        });
        applyBootstrap(result.bootstrap);
        const message = formatBatchToastMessage("import", result);
        setRecordingActionMessage(message);
        // A batch where every file failed (an unconvertible format, no ffmpeg)
        // can still come back with a non-error status. Nothing landed, so it is
        // not success — warn rather than show a green check.
        const importedCount = result.items.filter(
          (item) => item.status === "success",
        ).length;
        if (importedCount === 0) {
          showWarning(message);
        } else {
          notifyBatchResult(result, message);
        }
        return result;
      } catch (error) {
        const message = errorMessage(error, "The files could not be imported.");
        showWarning(message);
        setLoadError(message);
        return null;
      } finally {
        setBusyAction(null);
      }
    },
    [
      applyBootstrap,
      notifyBatchResult,
      persistSettingsIfNeeded,
      setBusyAction,
      setLoadError,
      setRecordingActionMessage,
      showWarning,
    ],
  );

  // YouTube import: fetch a video's audio with yt-dlp and register it with NO
  // transcript, exactly like importMedia — the file lands in the Library as
  // "Needs transcript". Returns the outcome rather than the bare batch: a
  // rejection (livestream, dead link, missing yt-dlp) has a reason worth showing
  // but no bootstrap, so it gets its own branch instead of a lossy null.
  const importYoutube = useCallback(
    async (url: string): Promise<YoutubeImportOutcome> => {
      const trimmed = url.trim();
      if (trimmed.length === 0) {
        return { ok: false, message: "No YouTube link was provided." };
      }

      try {
        setBusyAction("importYoutube");
        await persistSettingsIfNeeded();
        const result = await invoke<RecordingBatchResult>("import_youtube", {
          url: trimmed,
        });
        applyBootstrap(result.bootstrap);
        const message = formatBatchToastMessage("youtube", result);
        setRecordingActionMessage(message);
        // A fetch that failed (private/blocked video, missing yt-dlp) can still
        // come back with a non-error status. Nothing landed, so it is not a
        // success — warn rather than show a green check.
        const importedCount = result.items.filter(
          (item) => item.status === "success",
        ).length;
        if (importedCount === 0) {
          showWarning(message);
        } else {
          notifyBatchResult(result, message);
        }
        return { ok: true, result };
      } catch (error) {
        const message = errorMessage(
          error,
          "The YouTube link could not be imported.",
        );
        // Surface the failure as a transient toast only — matching transcription
        // and translation. Avoid `setLoadError`, which pins a permanent banner
        // (e.g. a rejected livestream would otherwise linger at the top).
        showWarning(message);
        return { ok: false, message };
      } finally {
        setBusyAction(null);
      }
    },
    [
      applyBootstrap,
      notifyBatchResult,
      persistSettingsIfNeeded,
      setBusyAction,
      setRecordingActionMessage,
      showWarning,
    ],
  );

  const addFuriganaToAnki = useCallback(
    async (filePaths: string[]) => {
      try {
        setBusyAction("addFurigana");
        await persistSettingsIfNeeded();
        const result = await invoke<RecordingBatchResult>("add_furigana_to_anki", {
          filePaths,
        });
        applyBootstrap(result.bootstrap);
        const message = formatBatchToastMessage("furigana", result);
        setRecordingActionMessage(message);
        if (result.status === "unavailable" || result.status === "partial") {
          showWarning(message);
        } else {
          showSuccess(message);
        }
      } catch (error) {
        const message = errorMessage(
          error,
          "Furigana could not be added to Anki cards.",
        );
        if (
          message.toLowerCase().includes("anki") ||
          message.toLowerCase().includes("furigana")
        ) {
          showWarning(message);
        }
        setLoadError(message);
      } finally {
        setBusyAction(null);
      }
    },
    [
      applyBootstrap,
      persistSettingsIfNeeded,
      setBusyAction,
      setLoadError,
      setRecordingActionMessage,
      showSuccess,
      showWarning,
    ],
  );

  // Transcription is no longer a blocking action here — it runs through the
  // frontend-driven, non-blocking `useTranscriptionQueue` (mirroring the YouTube
  // import queue) so the app stays usable while whisper-cli works.

  const translateRecordings = useCallback(
    // `force` bypasses the has-translation skip so a recording that is already
    // translated can be re-translated (deterministic overwrite of the sidecar).
    async (filePaths: string[], force = false) => {
      try {
        setBusyAction("translateRecording");
        const result = await invoke<RecordingBatchResult>("translate_recordings", {
          filePaths,
          force,
        });
        applyBootstrap(result.bootstrap);
        const message = formatBatchToastMessage("translate", result);
        setRecordingActionMessage(message);
        notifyBatchResult(result, message);
      } catch (error) {
        setLoadError(
          errorMessage(error, "The translation request could not be completed."),
        );
      } finally {
        setBusyAction(null);
      }
    },
    [
      applyBootstrap,
      notifyBatchResult,
      setBusyAction,
      setLoadError,
      setRecordingActionMessage,
    ],
  );

  const convertRecordingsToMp3 = useCallback(
    async (filePaths: string[]) => {
      const confirmed = await confirm({
        title: "Convert to MP3?",
        message: `Convert ${filePaths.length} recording${
          filePaths.length === 1 ? "" : "s"
        } to MP3? Wonder of U will keep the transcript/history, create MP3 files, and remove the original local WAV files after conversion succeeds. Existing Anki cards are not affected.`,
        okLabel: "Convert",
        cancelLabel: "Cancel",
        danger: true,
      });
      if (!confirmed) {
        return;
      }

      try {
        setBusyAction("convertMp3");
        const result = await invoke<RecordingBatchResult>(
          "convert_recordings_to_mp3",
          { filePaths },
        );
        applyBootstrap(result.bootstrap);
        const message = formatBatchToastMessage("convert", result);
        setRecordingActionMessage(message);
        if (result.status === "partial") {
          showWarning(message);
        } else {
          showSuccess(message);
        }
      } catch (error) {
        const message = errorMessage(
          error,
          "The selected recordings could not be converted to MP3.",
        );
        showWarning(message);
        setLoadError(message);
      } finally {
        setBusyAction(null);
      }
    },
    [
      applyBootstrap,
      setBusyAction,
      setLoadError,
      setRecordingActionMessage,
      showSuccess,
      showWarning,
    ],
  );

  return {
    addFuriganaToAnki,
    convertRecordingsToMp3,
    deleteRecording,
    deleteRecordings,
    importMedia,
    importYoutube,
    mineSegment,
    playRecording,
    pushRecordingsToAnki,
    translateRecordings,
  };
}
