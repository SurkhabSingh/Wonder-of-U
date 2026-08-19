import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { APP_SNAPSHOT_EVENT, DEFAULT_BOOTSTRAP } from "../constants";
import { mergeSettings } from "./mergeSettings";
import { transcriptionReady } from "../types";
import type {
  SettingsUpdate,
  AppBootstrap,
  AppSettings,
  AutosaveState,
} from "../types";

export function useAppBootstrap() {
  const [bootstrap, setBootstrap] = useState<AppBootstrap>(DEFAULT_BOOTSTRAP);
  const [settingsDraft, setSettingsDraft] = useState<AppSettings>(
    DEFAULT_BOOTSTRAP.settings,
  );
  const [autosaveState, setAutosaveState] = useState<AutosaveState>("idle");
  const [autosaveMessage, setAutosaveMessage] = useState(
    "Changes save automatically.",
  );
  const [loadError, setLoadError] = useState("");
  const settingsDirtyRef = useRef(false);
  const currentDraftKeyRef = useRef("");
  const latestTransitionCountRef = useRef(
    DEFAULT_BOOTSTRAP.shell.transitionCount,
  );
  const recordingToastStateRef = useRef({
    phase: DEFAULT_BOOTSTRAP.shell.phase,
    transitionCount: DEFAULT_BOOTSTRAP.shell.transitionCount,
  });
  // `transcriptionReady` starts null rather than false: the placeholder bootstrap carries an
  // empty requirement list, and "not yet known" must not read as a false→true edge the moment
  // the real answer arrives.
  const downloadToastStateRef = useRef<{
    status: string;
    transcriptionReady: boolean | null;
  }>({
    status: DEFAULT_BOOTSTRAP.modelDownload.status,
    transcriptionReady: null,
  });

  const settingsDraftKey = useMemo(
    () => JSON.stringify(settingsDraft),
    [settingsDraft],
  );
  const savedSettingsKey = useMemo(
    () => JSON.stringify(bootstrap.settings),
    [bootstrap.settings],
  );
  const settingsDirty = settingsDraftKey !== savedSettingsKey;

  useEffect(() => {
    settingsDirtyRef.current = settingsDirty;
    currentDraftKeyRef.current = settingsDraftKey;
  }, [settingsDirty, settingsDraftKey]);

  const applyBootstrap = useCallback(
    (nextBootstrap: AppBootstrap, options?: { preserveDraft?: boolean }) => {
      if (
        nextBootstrap.shell.transitionCount <
        latestTransitionCountRef.current
      ) {
        return false;
      }

      latestTransitionCountRef.current =
        nextBootstrap.shell.transitionCount;
      setBootstrap(nextBootstrap);
      if (!options?.preserveDraft) {
        setSettingsDraft(nextBootstrap.settings);
      }
      setLoadError("");
      return true;
    },
    [],
  );

  const syncRecordingToastState = useCallback(
    (nextBootstrap: AppBootstrap, options?: { notify?: boolean }) => {
      const previous = recordingToastStateRef.current;
      const next = {
        phase: nextBootstrap.shell.phase,
        transitionCount: nextBootstrap.shell.transitionCount,
      };

      recordingToastStateRef.current = next;

      if (
        !options?.notify ||
        (previous.phase === next.phase &&
          previous.transitionCount === next.transitionCount)
      ) {
        return;
      }

      const previousPhase = previous.phase;
      const nextPhase = next.phase;
      const recordingName =
        nextBootstrap.shell.currentRecordingName?.trim() || "Recording";

      if (nextPhase === "recording" && previousPhase !== "recording") {
        toast.success("Recording started", {
          description: recordingName,
          duration: 2500,
        });
        return;
      }

      if (nextPhase === "saving" && previousPhase === "recording") {
        toast("Recording stopped", {
          description: "Saving and processing the audio.",
          duration: 2500,
        });
        return;
      }

      if (
        nextPhase === "idle" &&
        (previousPhase === "saving" ||
          previousPhase === "transcribing" ||
          previousPhase === "recording")
      ) {
        const detail = nextBootstrap.shell.statusText;
        // A transcription is not a recording, and this fires for both. Titling every
        // return to idle "Recording saved" told a user who had just transcribed that
        // something had been recorded and saved, neither of which happened.
        const title =
          previousPhase === "transcribing" ? "Transcription finished" : "Recording saved";

        // A stop the user asked for is not good news. The backend now names it in the same
        // status line, so read it rather than inventing a second source of truth: a green
        // check on a cancelled run is exactly the report that was wrong before.
        if (detail.toLowerCase().includes("cancelled")) {
          toast(previousPhase === "transcribing" ? "Transcription cancelled" : "Cancelled", {
            description: detail,
            duration: 3500,
          });
          return;
        }

        toast.success(title, {
          description: detail,
          duration: 3500,
        });
        return;
      }

      if (nextPhase === "error" && previousPhase !== "error") {
        toast.error("Recording failed", {
          description: nextBootstrap.shell.statusText,
          duration: 5000,
        });
      }
    },
    [],
  );

  /**
   * Reports how a download ended.
   *
   * A download runs on its own thread and reports only into the shared snapshot, so a failure
   * reached no toast and no banner — the only trace was a progress card on a settings page
   * nobody was necessarily looking at. Now that the landing page can start one, that silence
   * is not survivable.
   *
   * Cancelling is the user's own doing and needs no report.
   */
  const syncDownloadToastState = useCallback(
    (nextBootstrap: AppBootstrap, options?: { notify?: boolean }) => {
      const previous = downloadToastStateRef.current;
      const next = {
        status: nextBootstrap.modelDownload.status,
        transcriptionReady: transcriptionReady(
          nextBootstrap.transcriptionRequirements,
        ),
      };
      downloadToastStateRef.current = next;

      if (!options?.notify) {
        return;
      }

      if (next.status === "failed" && previous.status !== "failed") {
        toast.error("Download failed", {
          description: nextBootstrap.modelDownload.message,
          duration: 5000,
        });
        return;
      }

      // `=== false` rather than `!previous`: null is "not yet known", and treating it as
      // not-ready would fire this on the first real snapshot for an install that was already
      // set up before launch.
      //
      // KNOWN, AND DELIBERATELY LEFT (audit 2026-08-19): this can fire with no download
      // involved. The speech-detector requirement is satisfied either by the file existing OR
      // by the audio type being "music", which skips VAD entirely — so with Whisper and FFmpeg
      // ready and the detector file absent, switching that dropdown to Music flips every
      // requirement ready at once and toasts here. The sentence is TRUE, which is why it stays:
      // gating on `modelDownload.status === "completed"` would only half-fix it (a download
      // earlier in the same session still leaves that status set) and would also silence the
      // toast for someone who pointed at an existing model by hand. Reads odd, says nothing
      // false.
      if (next.transcriptionReady && previous.transcriptionReady === false) {
        toast.success("Ready to transcribe", {
          description: "Your recordings can be transcribed now.",
          duration: 3500,
        });
      }
    },
    [],
  );

  useEffect(() => {
    let mounted = true;

    async function loadBootstrap() {
      try {
        const nextBootstrap = await invoke<AppBootstrap>("get_app_bootstrap");
        if (!mounted) {
          return;
        }

        applyBootstrap(nextBootstrap);
        syncRecordingToastState(nextBootstrap);
        syncDownloadToastState(nextBootstrap);
        setAutosaveState("idle");
        setAutosaveMessage("Changes save automatically.");
      } catch (error) {
        if (!mounted) {
          return;
        }

        setLoadError(
          error instanceof Error
            ? error.message
            : "The Wonder of U desktop state could not be loaded.",
        );
      }
    }

    void loadBootstrap();

    const unlistenPromise = listen<AppBootstrap>(APP_SNAPSHOT_EVENT, (event) => {
      const accepted = applyBootstrap(event.payload, {
        preserveDraft: settingsDirtyRef.current,
      });
      if (accepted) {
        syncRecordingToastState(event.payload, { notify: true });
        syncDownloadToastState(event.payload, { notify: true });
      }
    });

    return () => {
      mounted = false;
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [applyBootstrap, syncDownloadToastState, syncRecordingToastState]);

  useEffect(() => {
    if (!settingsDirty) {
      if (autosaveState !== "error") {
        setAutosaveState("idle");
        setAutosaveMessage("Changes save automatically.");
      }
      return;
    }

    const draftKeyAtSchedule = settingsDraftKey;
    const timer = window.setTimeout(async () => {
      try {
        setAutosaveState("saving");
        setAutosaveMessage("Saving changes...");
        const nextBootstrap = await invoke<AppBootstrap>("save_settings", {
          settings: settingsDraft,
        });
        const preserveDraft = currentDraftKeyRef.current !== draftKeyAtSchedule;
        applyBootstrap(nextBootstrap, { preserveDraft });
        if (!preserveDraft) {
          setAutosaveState("idle");
          setAutosaveMessage("All changes saved.");
        }
      } catch (error) {
        setAutosaveState("error");
        setAutosaveMessage(
          error instanceof Error
            ? error.message
            : "The updated settings could not be saved.",
        );
      }
    }, 320);

    return () => {
      window.clearTimeout(timer);
    };
  }, [applyBootstrap, settingsDraft, settingsDraftKey, settingsDirty]);

  const updateSettings = useCallback((update: SettingsUpdate) => {
    setSettingsDraft((current) => mergeSettings(current, update));
  }, []);

  const persistSettingsIfNeeded = useCallback(async () => {
    if (!settingsDirty) {
      return;
    }

    try {
      const draftKeyAtSave = currentDraftKeyRef.current;
      setAutosaveState("saving");
      setAutosaveMessage("Saving changes...");
      const nextBootstrap = await invoke<AppBootstrap>("save_settings", {
        settings: settingsDraft,
      });
      const preserveDraft = currentDraftKeyRef.current !== draftKeyAtSave;
      applyBootstrap(nextBootstrap, { preserveDraft });
      if (!preserveDraft) {
        setAutosaveState("idle");
        setAutosaveMessage("All changes saved.");
      }
    } catch (error) {
      setAutosaveState("error");
      setAutosaveMessage(
        error instanceof Error
          ? error.message
          : "The updated settings could not be saved.",
      );
      throw error;
    }
  }, [applyBootstrap, settingsDirty, settingsDraft]);

  return {
    applyBootstrap,
    autosaveMessage,
    autosaveState,
    bootstrap,
    loadError,
    persistSettingsIfNeeded,
    setBootstrap,
    setLoadError,
    settingsDraft,
    updateSettings,
  };
}
