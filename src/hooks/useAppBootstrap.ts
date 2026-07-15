import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { APP_SNAPSHOT_EVENT, DEFAULT_BOOTSTRAP } from "../constants";
import type {
  AnkiFieldMapping,
  AnkiSettings,
  AppBootstrap,
  AppSettings,
  AutosaveState,
  FeatureSettings,
  TranslationSettings,
  WhisperSettings,
} from "../types";

type SettingsUpdate = Partial<
  Omit<AppSettings, "features" | "whisper" | "anki" | "translation">
> & {
  features?: Partial<FeatureSettings>;
  whisper?: Partial<WhisperSettings>;
  translation?: Partial<TranslationSettings>;
  anki?: Partial<Omit<AnkiSettings, "fields">> & {
    fields?: Partial<AnkiFieldMapping>;
  };
};

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
        toast.success("Recording saved", {
          description: nextBootstrap.shell.statusText,
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
      }
    });

    return () => {
      mounted = false;
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [applyBootstrap, syncRecordingToastState]);

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
    setSettingsDraft((current) => {
      const nextFeatures: FeatureSettings = {
        ...current.features,
        ...(update.features ?? {}),
      };
      const nextWhisper: WhisperSettings = {
        ...current.whisper,
        ...(update.whisper ?? {}),
      };
      const nextTranslation: TranslationSettings = {
        ...current.translation,
        ...(update.translation ?? {}),
      };
      const nextAnki: AnkiSettings = {
        ...current.anki,
        ...(update.anki ?? {}),
        fields: {
          ...current.anki.fields,
          ...(update.anki?.fields ?? {}),
        },
      };

      return {
        ...current,
        ...update,
        whisper: nextWhisper,
        translation: nextTranslation,
        anki: nextAnki,
        features: nextFeatures,
      };
    });
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
