import { useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { AppBootstrap, BusyAction } from "../types";

type UseRecorderActionsOptions = {
  applyBootstrap: (
    nextBootstrap: AppBootstrap,
    options?: { preserveDraft?: boolean },
  ) => void;
  persistSettingsIfNeeded: () => Promise<void>;
  setBootstrap: (
    update: AppBootstrap | ((current: AppBootstrap) => AppBootstrap),
  ) => void;
  setBusyAction: (busyAction: BusyAction) => void;
  setLoadError: (message: string) => void;
};

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error ? error.message : fallback;
}

export function useRecorderActions({
  applyBootstrap,
  persistSettingsIfNeeded,
  setBootstrap,
  setBusyAction,
  setLoadError,
}: UseRecorderActionsOptions) {
  const startRecording = useCallback(async () => {
    try {
      setBusyAction("start");
      setBootstrap((current) => ({
        ...current,
        shell: {
          ...current.shell,
          phase: "recording",
          statusText: "Starting system audio capture...",
          startedAtMs: Date.now(),
          currentRecordingName: "Starting recording",
          lastOutputPath: null,
          lastTranscriptPath: null,
          transitionCount: current.shell.transitionCount + 1,
        },
      }));
      await persistSettingsIfNeeded();
      const nextBootstrap = await invoke<AppBootstrap>("start_recording", {
        requestedName: null,
      });
      applyBootstrap(nextBootstrap);
    } catch (error) {
      try {
        applyBootstrap(await invoke<AppBootstrap>("get_app_bootstrap"));
      } catch {
        // Keep the original startup error visible if recovery snapshot loading fails.
      }
      setLoadError(errorMessage(error, "Recording could not be started."));
    } finally {
      setBusyAction(null);
    }
  }, [applyBootstrap, persistSettingsIfNeeded, setBootstrap, setBusyAction, setLoadError]);

  const stopRecording = useCallback(async () => {
    try {
      setBusyAction("stop");
      const nextBootstrap = await invoke<AppBootstrap>("stop_recording");
      applyBootstrap(nextBootstrap);
    } catch (error) {
      setLoadError(errorMessage(error, "Recording could not be stopped."));
    } finally {
      setBusyAction(null);
    }
  }, [applyBootstrap, setBusyAction, setLoadError]);

  const hideToTray = useCallback(async () => {
    try {
      setBusyAction("hide");
      await invoke("hide_main_window");
    } catch (error) {
      setLoadError(errorMessage(error, "The window could not be hidden to the tray."));
    } finally {
      setBusyAction(null);
    }
  }, [setBusyAction, setLoadError]);

  return {
    hideToTray,
    startRecording,
    stopRecording,
  };
}
