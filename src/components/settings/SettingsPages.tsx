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
import { ScannerSettingsPage } from "./ScannerSettingsPage";
import { AnkiMappingSettingsPage } from "./AnkiMappingSettingsPage";
import { PreferencesSettingsPage } from "./PreferencesSettingsPage";
import type {
  BrowseDirectoryField,
  BrowseFileField,
  SettingsUpdate,
} from "./settingsTypes";
import { StorageSettingsPage } from "./StorageSettingsPage";
import { TranslationSettingsPage } from "./TranslationSettingsPage";
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
    // Sections collapse when they have nothing that needs doing, so a deep link can land
    // on a heading with its answer folded away. Opening it by clicking the summary rather
    // than by setting `open` keeps each disclosure's own state in step — the same path a
    // reader would take.
    target
      ?.querySelectorAll<HTMLElement>(
        // Section disclosures only. Whisper nests a further `<details>` for its advanced
        // controls, and a deep link should not fling that open as well.
        ".settings-disclosure:not([open]) > .settings-disclosure-summary",
      )
      .forEach((summary) => summary.click());
    target?.scrollIntoView({ behavior: "smooth", block: "start" });
    onScrollTargetHandled();
  }, [activePage, scrollTarget, onScrollTargetHandled]);

  if (activePage !== "settings") {
    return null;
  }

  return (
    <div className="settings-scroll">
      <article className="panel settings-surface">
        <section id="settings-preferences" className="settings-section">
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

        <section id="settings-translation" className="settings-section">
          <TranslationSettingsPage
            onUpdateSettings={onUpdateSettings}
            settingsDraft={settingsDraft}
          />
        </section>

        <section id="settings-storage" className="settings-section">
          <StorageSettingsPage
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

        <section id="settings-scanner" className="settings-section">
          <ScannerSettingsPage
            settingsDraft={settingsDraft}
            onUpdateSettings={onUpdateSettings}
          />
        </section>
      </article>
    </div>
  );
}
