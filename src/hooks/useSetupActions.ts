import { useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { RECOMMENDED_RUNTIME_VERSION } from "../constants";
import { errorMessage } from "../lib/errors";
import { normalizeSelection } from "../lib/helpers";
import type {
  AnkiFieldMapping,
  SettingsUpdate,
  AppBootstrap,
  AppSettings,
  BusyAction,
  KnownWordsSnapshot,
  SettingsSection,
  VocabularySuggestions,
  WhisperAssetUpdateResult,
} from "../types";

type UseSetupActionsOptions = {
  applyBootstrap: (nextBootstrap: AppBootstrap) => void;
  persistSettingsIfNeeded: () => Promise<void>;
  resolvedCliPath: string;
  resolvedModelPath: string;
  openSettingsSection: (section: SettingsSection) => void;
  setBusyAction: (busyAction: BusyAction) => void;
  setLoadError: (message: string) => void;
  setModelUpdateResult: (result: WhisperAssetUpdateResult | null) => void;
  setRuntimeUpdateResult: (result: WhisperAssetUpdateResult | null) => void;
  setYtdlpUpdateResult: (result: WhisperAssetUpdateResult | null) => void;
  settingsDraft: AppSettings;
  updateSettings: (update: SettingsUpdate) => void;
};

export function useSetupActions({
  applyBootstrap,
  persistSettingsIfNeeded,
  resolvedCliPath,
  resolvedModelPath,
  openSettingsSection,
  setBusyAction,
  setLoadError,
  setModelUpdateResult,
  setRuntimeUpdateResult,
  setYtdlpUpdateResult,
  settingsDraft,
  updateSettings,
}: UseSetupActionsOptions) {
  const downloadRuntimeVersion = useCallback(
    async (runtimeVersion: string) => {
      try {
        setBusyAction("downloadRuntime");
        setRuntimeUpdateResult(null);
        await persistSettingsIfNeeded();
        const nextBootstrap = await invoke<AppBootstrap>(
          "download_whisper_runtime_version",
          { runtimeVersion },
        );
        applyBootstrap(nextBootstrap);
        openSettingsSection("whisper");
      } catch (error) {
        setLoadError(
          errorMessage(error, "The selected Whisper runtime could not be prepared."),
        );
      } finally {
        setBusyAction(null);
      }
    },
    [
      applyBootstrap,
      persistSettingsIfNeeded,
      openSettingsSection,
      setBusyAction,
      setLoadError,
      setRuntimeUpdateResult,
    ],
  );

  const downloadRecommendedRuntime = useCallback(async () => {
    await downloadRuntimeVersion(RECOMMENDED_RUNTIME_VERSION);
  }, [downloadRuntimeVersion]);

  const downloadRecommendedFfmpeg = useCallback(async () => {
    try {
      setBusyAction("downloadFfmpeg");
      const nextBootstrap = await invoke<AppBootstrap>("download_recommended_ffmpeg");
      applyBootstrap(nextBootstrap);
      openSettingsSection("storage");
    } catch (error) {
      setLoadError(errorMessage(error, "FFmpeg could not be prepared."));
    } finally {
      setBusyAction(null);
    }
  }, [applyBootstrap, openSettingsSection, setBusyAction, setLoadError]);

  const downloadRecommendedYtdlp = useCallback(async () => {
    try {
      setBusyAction("downloadYtdlp");
      const nextBootstrap = await invoke<AppBootstrap>("download_recommended_ytdlp");
      applyBootstrap(nextBootstrap);
      openSettingsSection("storage");
    } catch (error) {
      setLoadError(errorMessage(error, "yt-dlp could not be prepared."));
    } finally {
      setBusyAction(null);
    }
  }, [applyBootstrap, openSettingsSection, setBusyAction, setLoadError]);

  const downloadRecommendedAlass = useCallback(async () => {
    try {
      setBusyAction("downloadAlass");
      const nextBootstrap = await invoke<AppBootstrap>("download_recommended_alass");
      applyBootstrap(nextBootstrap);
      openSettingsSection("storage");
    } catch (error) {
      setLoadError(errorMessage(error, "alass could not be prepared."));
    } finally {
      setBusyAction(null);
    }
  }, [applyBootstrap, openSettingsSection, setBusyAction, setLoadError]);

  const downloadRecommendedDictionary = useCallback(async () => {
    try {
      setBusyAction("downloadDictionary");
      const nextBootstrap = await invoke<AppBootstrap>(
        "download_recommended_dictionary",
      );
      applyBootstrap(nextBootstrap);
      openSettingsSection("studyPicks");
    } catch (error) {
      setLoadError(
        errorMessage(error, "The Japanese dictionary could not be prepared."),
      );
    } finally {
      setBusyAction(null);
    }
  }, [applyBootstrap, openSettingsSection, setBusyAction, setLoadError]);

  /**
   * Reads the collection and rebuilds the known-word list.
   *
   * The snapshot the command returns is discarded on purpose: it also emits an
   * app snapshot, and taking the result here as well would mean two paths writing
   * the same status, which is how the count in the header and the count in the
   * card end up disagreeing.
   */
  const refreshKnownWords = useCallback(async () => {
    try {
      setBusyAction("refreshKnownWords");
      // Settings first: the refresh reads the sources and threshold from the
      // SAVED settings, so an unsaved edit would otherwise rebuild the old list
      // and look like the change did nothing.
      await persistSettingsIfNeeded();
      await invoke<KnownWordsSnapshot>("refresh_known_words");
    } catch (error) {
      setLoadError(
        errorMessage(error, "Your known-word list could not be rebuilt."),
      );
    } finally {
      setBusyAction(null);
    }
  }, [persistSettingsIfNeeded, setBusyAction, setLoadError]);

  /**
   * Looks through the collection for note types that hold vocabulary.
   *
   * Returns the suggestions rather than applying them. Writing them straight into
   * settings would be the one thing this feature must not do: a wrong field fails
   * silently, so the user has to see the samples and choose.
   */
  const scanVocabularySources =
    useCallback(async (): Promise<VocabularySuggestions | null> => {
      try {
        setBusyAction("scanVocabulary");
        return await invoke<VocabularySuggestions>("scan_vocabulary_sources");
      } catch (error) {
        setLoadError(
          errorMessage(error, "Your Anki collection could not be searched."),
        );
        return null;
      } finally {
        setBusyAction(null);
      }
    }, [setBusyAction, setLoadError]);

  const checkYtdlpUpdate = useCallback(async () => {
    try {
      setBusyAction("checkYtdlpUpdate");
      const result = await invoke<WhisperAssetUpdateResult>("check_ytdlp_update");
      setYtdlpUpdateResult(result);
    } catch (error) {
      setLoadError(
        errorMessage(error, "The yt-dlp update check could not be completed."),
      );
    } finally {
      setBusyAction(null);
    }
  }, [setBusyAction, setLoadError, setYtdlpUpdateResult]);

  const downloadRecommendedModel = useCallback(async () => {
    try {
      setBusyAction("downloadModel");
      setModelUpdateResult(null);
      await persistSettingsIfNeeded();
      const nextBootstrap = await invoke<AppBootstrap>(
        "download_recommended_whisper_model",
      );
      applyBootstrap(nextBootstrap);
      openSettingsSection("whisper");
    } catch (error) {
      setLoadError(
        errorMessage(error, "The recommended Whisper model could not be prepared."),
      );
    } finally {
      setBusyAction(null);
    }
  }, [
    applyBootstrap,
    persistSettingsIfNeeded,
    openSettingsSection,
    setBusyAction,
    setLoadError,
    setModelUpdateResult,
  ]);

  const checkRuntimeUpdate = useCallback(async () => {
    try {
      setBusyAction("checkRuntimeUpdate");
      await persistSettingsIfNeeded();
      const result = await invoke<WhisperAssetUpdateResult>(
        "check_whisper_runtime_update",
      );
      setRuntimeUpdateResult(result);
    } catch (error) {
      setLoadError(
        errorMessage(error, "The runtime update check could not be completed."),
      );
    } finally {
      setBusyAction(null);
    }
  }, [persistSettingsIfNeeded, setBusyAction, setLoadError, setRuntimeUpdateResult]);

  const checkModelUpdate = useCallback(async () => {
    try {
      setBusyAction("checkModelUpdate");
      await persistSettingsIfNeeded();
      const result = await invoke<WhisperAssetUpdateResult>(
        "check_whisper_model_update",
      );
      setModelUpdateResult(result);
    } catch (error) {
      setLoadError(
        errorMessage(error, "The model update check could not be completed."),
      );
    } finally {
      setBusyAction(null);
    }
  }, [persistSettingsIfNeeded, setBusyAction, setLoadError, setModelUpdateResult]);

  const toggleDownloadPause = useCallback(async () => {
    try {
      const nextBootstrap = await invoke<AppBootstrap>(
        "toggle_whisper_model_download_pause",
      );
      applyBootstrap(nextBootstrap);
    } catch (error) {
      setLoadError(
        errorMessage(error, "The active download could not be paused or resumed."),
      );
    }
  }, [applyBootstrap, setLoadError]);

  const cancelDownload = useCallback(async () => {
    try {
      const nextBootstrap = await invoke<AppBootstrap>(
        "cancel_whisper_model_download",
      );
      applyBootstrap(nextBootstrap);
    } catch (error) {
      setLoadError(errorMessage(error, "The active download could not be cancelled."));
    }
  }, [applyBootstrap, setLoadError]);

  const browseForDirectory = useCallback(
    async (field: "outputDirectory" | "assetDirectory") => {
      try {
        setBusyAction("browse");
        const selection = normalizeSelection(
          await open({
            directory: true,
            multiple: false,
            defaultPath: settingsDraft[field] || undefined,
          }),
        );

        if (!selection) {
          return;
        }

        updateSettings({ [field]: selection });
      } catch (error) {
        setLoadError(errorMessage(error, "The folder chooser could not be opened."));
      } finally {
        setBusyAction(null);
      }
    },
    [setBusyAction, setLoadError, settingsDraft, updateSettings],
  );

  const browseForFile = useCallback(
    async (field: "cliPath" | "modelPath") => {
      try {
        setBusyAction("browse");
        const defaultPath =
          field === "cliPath" ? resolvedCliPath : resolvedModelPath;
        const selection = normalizeSelection(
          await open({
            directory: false,
            multiple: false,
            defaultPath: defaultPath || undefined,
          }),
        );

        if (!selection) {
          return;
        }

        updateSettings({ whisper: { [field]: selection } });
      } catch (error) {
        setLoadError(errorMessage(error, "The file chooser could not be opened."));
      } finally {
        setBusyAction(null);
      }
    },
    [
      resolvedCliPath,
      resolvedModelPath,
      setBusyAction,
      setLoadError,
      updateSettings,
    ],
  );

  const updateAnkiField = useCallback(
    (field: keyof AnkiFieldMapping, value: string) => {
      updateSettings({
        anki: {
          fields: {
            [field]: value,
          },
        },
      });
    },
    [updateSettings],
  );

  return {
    browseForDirectory,
    browseForFile,
    cancelDownload,
    checkModelUpdate,
    checkRuntimeUpdate,
    checkYtdlpUpdate,
    downloadRecommendedFfmpeg,
    downloadRecommendedModel,
    downloadRecommendedRuntime,
    downloadRecommendedYtdlp,
    downloadRecommendedAlass,
    downloadRecommendedDictionary,
    downloadRuntimeVersion,
    refreshKnownWords,
    scanVocabularySources,
    toggleDownloadPause,
    updateAnkiField,
  };
}
