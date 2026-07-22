import { LANGUAGE_OPTIONS, MODEL_OPTIONS } from "../../constants";
import type {
  AppBootstrap,
  AppSettings,
  BusyAction,
  WhisperAssetUpdateResult,
} from "../../types";
import { ThemedSelect } from "../ui/ThemedSelect";
import { TooltipBadge } from "../ui/Tooltip";
import { UpdateResultCard } from "../ui/UpdateResultCard";
import { DownloadProgressCard } from "./DownloadProgressCard";
import type { BrowseFileField, SettingsUpdate } from "./settingsTypes";

export function ModelSettingsPage({
  bootstrap,
  busyAction,
  downloadIsActive,
  modelInstalled,
  modelUpdateResult,
  onBrowseFile,
  onCancelDownload,
  onCheckModelUpdate,
  onDownloadRecommendedModel,
  onToggleDownloadPause,
  onUpdateSettings,
  resolvedModelPath,
  settingsDraft,
}: {
  bootstrap: AppBootstrap;
  busyAction: BusyAction;
  downloadIsActive: boolean;
  modelInstalled: boolean;
  modelUpdateResult: WhisperAssetUpdateResult | null;
  onBrowseFile: (field: BrowseFileField) => void | Promise<void>;
  onCancelDownload: () => void | Promise<void>;
  onCheckModelUpdate: () => void | Promise<void>;
  onDownloadRecommendedModel: () => void | Promise<void>;
  onToggleDownloadPause: () => void | Promise<void>;
  onUpdateSettings: (update: SettingsUpdate) => void;
  resolvedModelPath: string;
  settingsDraft: AppSettings;
}) {
  const selectedModel =
    MODEL_OPTIONS.find((option) => option.id === settingsDraft.whisper.modelChoice) ??
    MODEL_OPTIONS[2];
  const selectedLanguageCode = settingsDraft.whisper.language || "auto";
  const selectedLanguageKnown = LANGUAGE_OPTIONS.some(
    (option) => option.code === selectedLanguageCode,
  );
  const manualModelOverride = settingsDraft.whisper.modelPath.trim().length > 0;

  return (
    <div className="settings-subsection">
      <header className="panel-header">
        <h3>Whisper Model</h3>
        <TooltipBadge
          label="?"
          description="Choose a model file manually, or let the app download the recommended multilingual model into your selected asset folder."
        />
      </header>

      <div className="settings-grid">
        <label className="field">
          <span>Managed model</span>
          <ThemedSelect
            value={settingsDraft.whisper.modelChoice}
            options={MODEL_OPTIONS.map((option) => ({
              value: option.id,
              label: option.label,
            }))}
            placeholder="Managed model"
            onChange={(nextValue) =>
              onUpdateSettings({
                whisper: {
                  modelChoice: nextValue,
                },
              })
            }
            disabled={manualModelOverride}
            title={
              manualModelOverride
                ? "Clear the manual model override to use app-managed models."
                : "Choose the app-managed Whisper model."
            }
          />
        </label>

        <label className="field">
          <span>Language</span>
          <ThemedSelect
            value={selectedLanguageCode}
            options={[
              ...(!selectedLanguageKnown
                ? [
                    {
                      value: selectedLanguageCode,
                      label: `Custom (${selectedLanguageCode})`,
                    },
                  ]
                : []),
              ...LANGUAGE_OPTIONS.map((option) => ({
                value: option.code,
                label: `${option.label} (${option.code})`,
              })),
            ]}
            placeholder="Language"
            onChange={(nextValue) =>
              onUpdateSettings({
                whisper: {
                  language: nextValue,
                },
              })
            }
          />
        </label>

        <label className="field">
          <span>CPU usage during transcription</span>
          <ThemedSelect
            value={settingsDraft.whisper.cpuUsage || "balanced"}
            options={[
              { value: "low", label: "Low" },
              { value: "balanced", label: "Balanced" },
              { value: "high", label: "High" },
            ]}
            placeholder="CPU usage"
            onChange={(nextValue) =>
              onUpdateSettings({
                whisper: {
                  cpuUsage: nextValue,
                },
              })
            }
          />
        </label>

        <label className="field">
          <span>Audio type</span>
          <ThemedSelect
            value={settingsDraft.whisper.audioType || "speech"}
            options={[
              { value: "speech", label: "Speech" },
              { value: "music", label: "Music (songs)" },
            ]}
            placeholder="Audio type"
            onChange={(nextValue) =>
              onUpdateSettings({
                whisper: {
                  audioType: nextValue,
                },
              })
            }
          />
        </label>
      </div>

      <p className="microcopy">
        Higher uses more CPU cores for faster transcription; lower uses fewer, so
        the machine stays responsive while it runs.
      </p>

      <p className="microcopy">
        Set Audio type to Music for songs — it transcribes the whole song including
        sung vocals (Speech mode's voice detection drops singing). Timestamps are a
        little looser in Music mode; keep it on Speech for dialogue.
      </p>

      <div
        className="model-summary"
        title={
          manualModelOverride
            ? settingsDraft.whisper.modelPath
            : selectedModel.description
        }
      >
        <strong>
          {manualModelOverride ? "Manual model override" : selectedModel.label}
        </strong>
        {manualModelOverride ? (
          <span>The manual GGML model path is being used.</span>
        ) : (
          <span>
            {selectedModel.diskSize} - {selectedModel.memoryUsage} RAM
          </span>
        )}
      </div>

      <details className="disclosure">
        <summary>Manual model override</summary>
        <label className="field">
          <span>GGML model path</span>
          <div className="input-with-action">
            <input
              type="text"
              value={settingsDraft.whisper.modelPath}
              onChange={(event) =>
                onUpdateSettings({
                  whisper: {
                    modelPath: event.currentTarget.value,
                  },
                })
              }
              placeholder={resolvedModelPath || "GGML model path"}
            />
            <button
              type="button"
              className="ghost"
              onClick={() => void onBrowseFile("modelPath")}
              disabled={busyAction === "browse"}
            >
              Browse
            </button>
          </div>
        </label>
      </details>

      <div className="download-section">
        {modelInstalled ? (
          <div className="installed-card">
            <div className="installed-row">
              <strong>Model ready</strong>
              {bootstrap.whisperDetection.modelManaged ? (
                <div className="action-row inline-actions">
                  <button
                    type="button"
                    className="secondary"
                    onClick={() => void onCheckModelUpdate()}
                    disabled={busyAction === "checkModelUpdate"}
                  >
                    Check for Updates
                  </button>
                </div>
              ) : null}
            </div>
            <UpdateResultCard result={modelUpdateResult} />
          </div>
        ) : (
          <div className="action-row inline-actions">
            <button
              type="button"
              onClick={() => void onDownloadRecommendedModel()}
              disabled={downloadIsActive || busyAction === "downloadModel"}
            >
              Download {selectedModel.label} Model
            </button>
          </div>
        )}
        <DownloadProgressCard
          snapshot={bootstrap.modelDownload}
          kind="model"
          downloadIsActive={downloadIsActive}
          onTogglePause={() => void onToggleDownloadPause()}
          onCancel={() => void onCancelDownload()}
        />
      </div>

    </div>
  );
}
