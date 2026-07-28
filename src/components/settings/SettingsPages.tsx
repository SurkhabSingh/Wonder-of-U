import { useEffect } from "react";
import type {
  AnkiCatalog,
  AnkiFieldMapping,
  AppBootstrap,
  AppPage,
  AppSettings,
  AutosaveState,
  BusyAction,
  SettingsSection,
  WhisperAssetUpdateResult,
} from "../../types";
import type { RefreshAnkiCatalogOptions } from "../../hooks/useAnkiCatalog";
import { AnkiMappingSettingsPage } from "./AnkiMappingSettingsPage";
import { PreferencesSettingsPage } from "./PreferencesSettingsPage";
import type {
  BrowseDirectoryField,
  BrowseFileField,
  SettingsUpdate,
} from "./settingsTypes";
import { DownloadsSettingsPage } from "./DownloadsSettingsPage";
import { WhisperSettingsPage } from "./WhisperSettingsPage";

export function SettingsPages({
  activePage,
  scrollTarget,
  onScrollTargetHandled,
  bootstrap,
  settingsDraft,
  autosaveState,
  autosaveMessage,
  busyAction,
  displayedAnkiCatalog,
  activeRuntimeVersion,
  installedRuntimeVersions,
  runtimeUpdateResult,
  runtimeUpdateVersion,
  modelUpdateResult,
  ytdlpUpdateResult,
  runtimeInstalled,
  modelInstalled,
  resolvedCliPath,
  resolvedModelPath,
  downloadIsActive,
  onUpdateSettings,
  onBrowseDirectory,
  onBrowseFile,
  onCheckRuntimeUpdate,
  onDownloadRuntimeVersion,
  onDownloadRecommendedRuntime,
  onCheckModelUpdate,
  onDownloadRecommendedModel,
  onDownloadRecommendedFfmpeg,
  onDownloadRecommendedYtdlp,
  onDownloadRecommendedAlass,
  onCheckYtdlpUpdate,
  onToggleDownloadPause,
  onCancelDownload,
  onRefreshAnkiCatalog,
  onUpdateAnkiField,
}: {
  activePage: AppPage;
  scrollTarget: SettingsSection | null;
  onScrollTargetHandled: () => void;
  bootstrap: AppBootstrap;
  settingsDraft: AppSettings;
  autosaveState: AutosaveState;
  autosaveMessage: string;
  busyAction: BusyAction;
  displayedAnkiCatalog: AnkiCatalog;
  activeRuntimeVersion: string;
  installedRuntimeVersions: string[];
  runtimeUpdateResult: WhisperAssetUpdateResult | null;
  runtimeUpdateVersion: string | null;
  modelUpdateResult: WhisperAssetUpdateResult | null;
  ytdlpUpdateResult: WhisperAssetUpdateResult | null;
  runtimeInstalled: boolean;
  modelInstalled: boolean;
  resolvedCliPath: string;
  resolvedModelPath: string;
  downloadIsActive: boolean;
  onUpdateSettings: (update: SettingsUpdate) => void;
  onBrowseDirectory: (field: BrowseDirectoryField) => void | Promise<void>;
  onBrowseFile: (field: BrowseFileField) => void | Promise<void>;
  onCheckRuntimeUpdate: () => void | Promise<void>;
  onDownloadRuntimeVersion: (version: string) => void | Promise<void>;
  onDownloadRecommendedRuntime: () => void | Promise<void>;
  onCheckModelUpdate: () => void | Promise<void>;
  onDownloadRecommendedModel: () => void | Promise<void>;
  onDownloadRecommendedFfmpeg: () => void | Promise<void>;
  onDownloadRecommendedYtdlp: () => void | Promise<void>;
  onDownloadRecommendedAlass: () => void | Promise<void>;
  onCheckYtdlpUpdate: () => void | Promise<void>;
  onToggleDownloadPause: () => void | Promise<void>;
  onCancelDownload: () => void | Promise<void>;
  onRefreshAnkiCatalog: (
    noteType?: string,
    options?: RefreshAnkiCatalogOptions,
  ) => void | Promise<void>;
  onUpdateAnkiField: (field: keyof AnkiFieldMapping, value: string) => void;
}) {
  // Deep links from the Setup checklist (and post-download navigation) land on
  // the settings page and ask a specific section to scroll into view.
  useEffect(() => {
    if (activePage !== "settings" || scrollTarget === null) {
      return;
    }
    const target = document.getElementById(`settings-${scrollTarget}`);
    // A target is either a whole section or a group inside one, and either way what has to
    // open is the disclosure enclosing it — `closest` for a group, `querySelector` for a
    // section, which is its own ancestor of nothing. Opening it by clicking the summary
    // rather than by setting `open` keeps the disclosure's own state in step, the same path
    // a reader would take. Whisper nests a further `<details>` for advanced controls; only
    // the enclosing section disclosure is touched, never that one.
    const disclosure =
      target?.closest<HTMLElement>(".settings-disclosure") ??
      target?.querySelector<HTMLElement>(".settings-disclosure");
    if (disclosure instanceof HTMLDetailsElement && !disclosure.open) {
      disclosure
        .querySelector<HTMLElement>(":scope > .settings-disclosure-summary")
        ?.click();
    }
    target?.scrollIntoView({ behavior: "smooth", block: "start" });
    onScrollTargetHandled();
  }, [activePage, scrollTarget, onScrollTargetHandled]);

  if (activePage !== "settings") {
    return null;
  }

  return (
    <div className="settings-scroll">
      <article className="panel settings-surface">
        <section className="settings-section">
          <PreferencesSettingsPage
            autosaveMessage={autosaveMessage}
            autosaveState={autosaveState}
            busyAction={busyAction}
            onBrowseDirectory={onBrowseDirectory}
            onUpdateSettings={onUpdateSettings}
            settingsDraft={settingsDraft}
          />
        </section>

        <section id="settings-whisper" className="settings-section">
          <WhisperSettingsPage
            activeRuntimeVersion={activeRuntimeVersion}
            bootstrap={bootstrap}
            busyAction={busyAction}
            downloadIsActive={downloadIsActive}
            installedRuntimeVersions={installedRuntimeVersions}
            modelInstalled={modelInstalled}
            modelUpdateResult={modelUpdateResult}
            onBrowseFile={onBrowseFile}
            onCancelDownload={onCancelDownload}
            onCheckModelUpdate={onCheckModelUpdate}
            onCheckRuntimeUpdate={onCheckRuntimeUpdate}
            onDownloadRecommendedModel={onDownloadRecommendedModel}
            onDownloadRecommendedRuntime={onDownloadRecommendedRuntime}
            onDownloadRuntimeVersion={onDownloadRuntimeVersion}
            onToggleDownloadPause={onToggleDownloadPause}
            onUpdateSettings={onUpdateSettings}
            resolvedCliPath={resolvedCliPath}
            resolvedModelPath={resolvedModelPath}
            runtimeInstalled={runtimeInstalled}
            runtimeUpdateResult={runtimeUpdateResult}
            runtimeUpdateVersion={runtimeUpdateVersion}
            settingsDraft={settingsDraft}
          />
        </section>

        <section id="settings-downloads" className="settings-section">
          <DownloadsSettingsPage
            bootstrap={bootstrap}
            busyAction={busyAction}
            downloadIsActive={downloadIsActive}
            ytdlpUpdateResult={ytdlpUpdateResult}
            onCancelDownload={onCancelDownload}
            onDownloadRecommendedFfmpeg={onDownloadRecommendedFfmpeg}
            onDownloadRecommendedYtdlp={onDownloadRecommendedYtdlp}
            onDownloadRecommendedAlass={onDownloadRecommendedAlass}
            onCheckYtdlpUpdate={onCheckYtdlpUpdate}
            onToggleDownloadPause={onToggleDownloadPause}
          />
        </section>

        <section id="settings-anki" className="settings-section">
          <AnkiMappingSettingsPage
            busyAction={busyAction}
            displayedAnkiCatalog={displayedAnkiCatalog}
            onRefreshAnkiCatalog={onRefreshAnkiCatalog}
            onUpdateAnkiField={onUpdateAnkiField}
            onUpdateSettings={onUpdateSettings}
            settingsDraft={settingsDraft}
          />
        </section>

      </article>
    </div>
  );
}
