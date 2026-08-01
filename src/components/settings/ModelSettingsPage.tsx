import { LANGUAGE_OPTIONS, MODEL_OPTIONS } from "../../constants";
import type {
  AppBootstrap,
  AppSettings,
  BusyAction,
  WhisperAssetUpdateResult,
} from "../../types";
import { isDownloadBusy } from "../../types";
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

        <label className="field">
          <span className="field-label-with-help">
            <span>Transcription speed</span>
            {/* The speed win is measured, but it was measured on two recordings — how the
                text shifts depends on the audio, so this is flagged rather than presented
                as a settled improvement. */}
            <span className="field-experimental-tag">Experimental</span>
            <TooltipBadge
              label="?"
              description="Faster narrows the decoder's search from whisper's 5-wide beam to a single greedy pass. On our test recordings it ran 13–23% quicker, and where the text differed it differed sideways — kana instead of kanji, a comma instead of a full stop, a sentence split a word earlier — rather than less accurately. That was two recordings; greedy decoding is generally more prone to repeating a line on difficult audio, so compare before trusting it on something you mine."
            />
          </span>
          <ThemedSelect
            value={settingsDraft.whisper.decodeSpeed || "balanced"}
            options={[
              // "Standard", not "Balanced": CPU usage two rows up already offers a
              // Balanced, and two speed-ish controls sharing a value label with unrelated
              // meanings is a needless trap. The stored value stays "balanced".
              { value: "balanced", label: "Standard (default)" },
              { value: "fast", label: "Faster" },
            ]}
            placeholder="Transcription speed"
            describedBy="transcription-speed-help"
            onChange={(nextValue) =>
              onUpdateSettings({
                whisper: {
                  decodeSpeed: nextValue,
                },
              })
            }
          />
        </label>
      </div>

      {/* Each paragraph names its own control. Without that, the reader takes whichever
          one sits nearest — and this one is about speed, directly under a speed setting
          whose options are not the "higher"/"lower" it describes. */}
      <p className="microcopy">
        CPU usage controls how much of the machine transcription may take: higher uses
        more cores and finishes sooner, lower uses fewer so the machine stays responsive
        while it runs.
      </p>

      <p className="microcopy">
        Set Audio type to Music for songs — it transcribes the whole song including
        sung vocals (Speech mode's voice detection drops singing). Timestamps are a
        little looser in Music mode; keep it on Speech for dialogue.
      </p>

      {/* The id is what the select's aria-describedby points at, so this caveat reaches a
          screen reader — the trigger's aria-label otherwise replaces the whole label and
          the Experimental badge with it. */}
      <p className="microcopy" id="transcription-speed-help">
        Transcription speed is <strong>experimental</strong>: Faster was 13&ndash;23%
        quicker on our test recordings without reading less accurately, but that was two
        recordings and your audio may behave differently. Compare both on a long episode
        before relying on it; Standard is unchanged from before this setting existed.
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
              disabled={downloadIsActive || isDownloadBusy(busyAction)}
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
