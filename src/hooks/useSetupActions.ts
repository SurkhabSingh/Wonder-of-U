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
  /**
   * Reports a failure as a toast.
   *
   * Every failure in this hook is the outcome of one thing the user just pressed, so every
   * one of them belongs here rather than in the app-wide error banner. The banner stays until
   * something else replaces it, which is right for a condition the app is in ("settings could
   * not be loaded") and wrong for an event that is already over: "There is no active model
   * download to pause or resume" sat across the top of the window for the rest of the session,
   * describing a download that had finished. The video library already drew this distinction;
   * setup did not.
   */
  showError: (message: string) => void;
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
  showError,
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
        showError(
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
      showError,
      setRuntimeUpdateResult,
    ],
  );

  const downloadRecommendedRuntime = useCallback(async () => {
    await downloadRuntimeVersion(RECOMMENDED_RUNTIME_VERSION);
  }, [downloadRuntimeVersion]);

  /**
   * Fetches whatever transcription still needs, in one press.
   *
   * Which downloads those are is decided in Rust, beside the readiness check that says whether
   * anything is missing at all — so this cannot ask for the wrong thing without the two
   * disagreeing at the one site that answers both. Unlike the six per-asset actions this does
   * not navigate: the point is progress without leaving Home.
   */
  const downloadMissingEssentials = useCallback(async () => {
    try {
      setBusyAction("downloadEssentials");
      // The model and runtime downloads read the SAVED settings, so an unsaved draft would
      // fetch for the previous choice.
      await persistSettingsIfNeeded();
      const nextBootstrap = await invoke<AppBootstrap>(
        "download_missing_essentials",
      );
      applyBootstrap(nextBootstrap);
    } catch (error) {
      showError(errorMessage(error, "The downloads could not be started."));
    } finally {
      setBusyAction(null);
    }
  }, [applyBootstrap, persistSettingsIfNeeded, setBusyAction, showError]);

  const downloadRecommendedFfmpeg = useCallback(async () => {
    try {
      setBusyAction("downloadFfmpeg");
      const nextBootstrap = await invoke<AppBootstrap>("download_recommended_ffmpeg");
      applyBootstrap(nextBootstrap);
      openSettingsSection("storage");
    } catch (error) {
      showError(errorMessage(error, "FFmpeg could not be prepared."));
    } finally {
      setBusyAction(null);
    }
  }, [applyBootstrap, openSettingsSection, setBusyAction, showError]);

  /**
   * Fetches a fresh FFmpeg over a working one.
   *
   * Its own action rather than a flag on the download above, because the two differ in what the
   * backend is allowed to skip: the plain download stops early when a runnable copy is present,
   * which is right for "I have none" and silently does nothing for "replace what I have".
   */
  const reinstallFfmpeg = useCallback(async () => {
    try {
      setBusyAction("reinstallFfmpeg");
      const nextBootstrap = await invoke<AppBootstrap>("reinstall_ffmpeg");
      applyBootstrap(nextBootstrap);
      openSettingsSection("storage");
    } catch (error) {
      showError(errorMessage(error, "FFmpeg could not be prepared."));
    } finally {
      setBusyAction(null);
    }
  }, [applyBootstrap, openSettingsSection, setBusyAction, showError]);

  const downloadRecommendedYtdlp = useCallback(async () => {
    try {
      setBusyAction("downloadYtdlp");
      const nextBootstrap = await invoke<AppBootstrap>("download_recommended_ytdlp");
      applyBootstrap(nextBootstrap);
      openSettingsSection("storage");
    } catch (error) {
      showError(errorMessage(error, "yt-dlp could not be prepared."));
    } finally {
      setBusyAction(null);
    }
  }, [applyBootstrap, openSettingsSection, setBusyAction, showError]);

  const downloadRecommendedAlass = useCallback(async () => {
    try {
      setBusyAction("downloadAlass");
      const nextBootstrap = await invoke<AppBootstrap>("download_recommended_alass");
      applyBootstrap(nextBootstrap);
      openSettingsSection("storage");
    } catch (error) {
      showError(errorMessage(error, "alass could not be prepared."));
    } finally {
      setBusyAction(null);
    }
  }, [applyBootstrap, openSettingsSection, setBusyAction, showError]);

  const downloadRecommendedDictionary = useCallback(async () => {
    try {
      setBusyAction("downloadDictionary");
      const nextBootstrap = await invoke<AppBootstrap>(
        "download_recommended_dictionary",
      );
      applyBootstrap(nextBootstrap);
      openSettingsSection("studyPicks");
    } catch (error) {
      showError(
        errorMessage(error, "The Japanese dictionary could not be prepared."),
      );
    } finally {
      setBusyAction(null);
    }
  }, [applyBootstrap, openSettingsSection, setBusyAction, showError]);

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
      showError(
        errorMessage(error, "Your known-word list could not be rebuilt."),
      );
    } finally {
      setBusyAction(null);
    }
  }, [persistSettingsIfNeeded, setBusyAction, showError]);

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
        showError(
          errorMessage(error, "Your Anki collection could not be searched."),
        );
        return null;
      } finally {
        setBusyAction(null);
      }
    }, [setBusyAction, showError]);

  const checkYtdlpUpdate = useCallback(async () => {
    try {
      setBusyAction("checkYtdlpUpdate");
      const result = await invoke<WhisperAssetUpdateResult>("check_ytdlp_update");
      setYtdlpUpdateResult(result);
    } catch (error) {
      showError(
        errorMessage(error, "The yt-dlp update check could not be completed."),
      );
    } finally {
      setBusyAction(null);
    }
  }, [setBusyAction, showError, setYtdlpUpdateResult]);

  /**
   * Fetches only the speech detector, for the repair shown when it is missing but the model
   * is not.
   *
   * Its own command rather than reusing the model download: that one writes to
   * `<asset_dir>/models/` and skips only what is already there, but a managed model is
   * accepted from six different directories and a manual path from anywhere — so "the model
   * is installed" does not mean "the model is where that download would put it", and pressing
   * a repair could have started a multi-gigabyte transfer.
   */
  const downloadWhisperVadModel = useCallback(async () => {
    try {
      setBusyAction("downloadModel");
      const nextBootstrap = await invoke<AppBootstrap>("download_whisper_vad_model");
      applyBootstrap(nextBootstrap);
    } catch (error) {
      showError(errorMessage(error, "The speech detector could not be prepared."));
    } finally {
      setBusyAction(null);
    }
  }, [applyBootstrap, setBusyAction, showError]);

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
      showError(
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
    showError,
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
      showError(
        errorMessage(error, "The runtime update check could not be completed."),
      );
    } finally {
      setBusyAction(null);
    }
  }, [persistSettingsIfNeeded, setBusyAction, showError, setRuntimeUpdateResult]);

  const checkModelUpdate = useCallback(async () => {
    try {
      setBusyAction("checkModelUpdate");
      await persistSettingsIfNeeded();
      const result = await invoke<WhisperAssetUpdateResult>(
        "check_whisper_model_update",
      );
      setModelUpdateResult(result);
    } catch (error) {
      showError(
        errorMessage(error, "The model update check could not be completed."),
      );
    } finally {
      setBusyAction(null);
    }
  }, [persistSettingsIfNeeded, setBusyAction, showError, setModelUpdateResult]);

  /**
   * Pauses or resumes the running download.
   *
   * Nothing is applied from the call, and the command returns nothing to apply — the same
   * reasoning as `refreshKnownWords` above, but with a sharper symptom. The download thread
   * emits a fresh app snapshot on every 64KB chunk, so a bootstrap built at the end of this
   * command could carry a "downloading" written microseconds after the pause was recorded,
   * and applying it here landed it *after* the worker's own "paused" had arrived. The button
   * then still read "Pause Download" over a paused download, so pressing it again resumed
   * rather than paused, and the transfer looked stuck part-way.
   *
   * Pause is where this surfaced because pause is where the emissions stop: the worker
   * announces "paused" once and then blocks, so nothing arrives afterwards to correct a bad
   * overwrite. Every other status keeps emitting and heals itself within a chunk.
   */
  const toggleDownloadPause = useCallback(async () => {
    try {
      await invoke("toggle_whisper_model_download_pause");
    } catch (error) {
      showError(
        errorMessage(error, "The active download could not be paused or resumed."),
      );
    }
  }, [showError]);

  /** Same as `toggleDownloadPause`: the emitted snapshot is the only writer. */
  const cancelDownload = useCallback(async () => {
    try {
      await invoke("cancel_whisper_model_download");
    } catch (error) {
      showError(errorMessage(error, "The active download could not be cancelled."));
    }
  }, [showError]);

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
        showError(errorMessage(error, "The folder chooser could not be opened."));
      } finally {
        setBusyAction(null);
      }
    },
    [setBusyAction, showError, settingsDraft, updateSettings],
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
        showError(errorMessage(error, "The file chooser could not be opened."));
      } finally {
        setBusyAction(null);
      }
    },
    [
      resolvedCliPath,
      resolvedModelPath,
      setBusyAction,
      showError,
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
    reinstallFfmpeg,
    downloadRecommendedModel,
    downloadWhisperVadModel,
    downloadRecommendedRuntime,
    downloadRecommendedYtdlp,
    downloadRecommendedAlass,
    downloadRecommendedDictionary,
    downloadMissingEssentials,
    downloadRuntimeVersion,
    refreshKnownWords,
    scanVocabularySources,
    toggleDownloadPause,
    updateAnkiField,
  };
}
