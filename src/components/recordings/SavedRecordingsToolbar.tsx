import * as DropdownMenuPrimitive from "@radix-ui/react-dropdown-menu";
import { MP3_CONVERSION_WARNING } from "../../constants";
import type { RecordingFilterTab } from "../../lib/navigation";
import type { BusyAction, RecentRecording, RecordingFilter } from "../../types";
import { TooltipWrap } from "../ui/Tooltip";
import { RecordingFilterTabs } from "./RecordingFilterTabs";
import { SelectionActionBar } from "./SelectionActionBar";

type RecordingAction = (filePaths: string[]) => void | Promise<void>;
type PushAction = (filePaths: string[], deckName?: string) => void | Promise<void>;

export function SavedRecordingsToolbar({
  recordingFilter,
  recordingFilterTabs,
  visibleSelectedPaths,
  configuredAnkiDeckLabel,
  configuredDeckMenuOptions,
  currentDeckName,
  busyAction,
  allowMp3Conversion,
  expressionFieldMapped,
  selectedUntranscribedRecordings,
  selectedPushableRecordings,
  selectedTranscribedRecordings,
  selectedFuriganaRecordings,
  selectedUntranslatedRecordings,
  selectedConvertibleRecordings,
  selectedRecordingsPushableToDeck,
  untranscribedRecordings,
  pushableRecordings,
  untranslatedRecordings,
  convertibleRecordings,
  onFilterChange,
  onDefaultDeckChange,
  onRefreshAnki,
  onTranscribe,
  onPushToAnki,
  onAddFurigana,
  onTranslate,
  onConvertToMp3,
  onDelete,
  onClearSelection,
}: {
  recordingFilter: RecordingFilter;
  recordingFilterTabs: RecordingFilterTab[];
  visibleSelectedPaths: string[];
  configuredAnkiDeckLabel: string;
  configuredDeckMenuOptions: string[];
  currentDeckName: string;
  busyAction: BusyAction;
  allowMp3Conversion: boolean;
  expressionFieldMapped: boolean;
  selectedUntranscribedRecordings: RecentRecording[];
  selectedPushableRecordings: RecentRecording[];
  selectedTranscribedRecordings: RecentRecording[];
  selectedFuriganaRecordings: RecentRecording[];
  selectedUntranslatedRecordings: RecentRecording[];
  selectedConvertibleRecordings: RecentRecording[];
  selectedRecordingsPushableToDeck: (deckName: string) => RecentRecording[];
  untranscribedRecordings: RecentRecording[];
  pushableRecordings: RecentRecording[];
  untranslatedRecordings: RecentRecording[];
  convertibleRecordings: RecentRecording[];
  onFilterChange: (filter: RecordingFilter) => void;
  onDefaultDeckChange: (deckName: string) => void;
  onRefreshAnki: () => void | Promise<void>;
  onTranscribe: RecordingAction;
  onPushToAnki: PushAction;
  onAddFurigana: RecordingAction;
  onTranslate: RecordingAction;
  onConvertToMp3: RecordingAction;
  onDelete: RecordingAction;
  onClearSelection: () => void;
}) {
  const hasSelection = visibleSelectedPaths.length > 0;

  // Gmail-style select mode: with a selection active the filter tabs + toolbar
  // row give way to a batch-actions bar that occupies the same slot. Nothing new
  // floats or docks — the toolbar row itself morphs into the action bar.
  if (hasSelection) {
    return (
      <SelectionActionBar
        visibleSelectedPaths={visibleSelectedPaths}
        configuredDeckMenuOptions={configuredDeckMenuOptions}
        busyAction={busyAction}
        allowMp3Conversion={allowMp3Conversion}
        expressionFieldMapped={expressionFieldMapped}
        selectedUntranscribedRecordings={selectedUntranscribedRecordings}
        selectedPushableRecordings={selectedPushableRecordings}
        selectedTranscribedRecordings={selectedTranscribedRecordings}
        selectedFuriganaRecordings={selectedFuriganaRecordings}
        selectedUntranslatedRecordings={selectedUntranslatedRecordings}
        selectedConvertibleRecordings={selectedConvertibleRecordings}
        selectedRecordingsPushableToDeck={selectedRecordingsPushableToDeck}
        onTranscribe={onTranscribe}
        onPushToAnki={onPushToAnki}
        onAddFurigana={onAddFurigana}
        onTranslate={onTranslate}
        onConvertToMp3={onConvertToMp3}
        onDelete={onDelete}
        onClearSelection={onClearSelection}
      />
    );
  }

  // No selection: the toolbar's job is filters + the "…All" shortcuts for the
  // current filter.
  return (
    <>
      <RecordingFilterTabs
        value={recordingFilter}
        tabs={recordingFilterTabs}
        onChange={onFilterChange}
      />

      <div className="recording-toolbar">
        {/* LEFT: the current filter's one bulk action, styled as a prominent
            accent-outline button. Empty when the active filter has none. */}
        <div className="recording-toolbar-actions">
          {recordingFilter === "needsTranscription" &&
          untranscribedRecordings.length > 0 ? (
            <button
              type="button"
              className="selection-action-btn"
              onClick={() =>
                void onTranscribe(
                  untranscribedRecordings.map((recording) => recording.filePath),
                )
              }
              disabled={busyAction === "transcribeRecording"}
            >
              Transcribe All
            </button>
          ) : null}
          {recordingFilter === "needsAnki" && pushableRecordings.length > 0 ? (
            <button
              type="button"
              className="selection-action-btn"
              onClick={() =>
                void onPushToAnki(
                  pushableRecordings.map((recording) => recording.filePath),
                )
              }
              disabled={busyAction === "pushAnki"}
            >
              Push All
            </button>
          ) : null}
          {recordingFilter === "needsTranslation" &&
          untranslatedRecordings.length > 0 ? (
            <button
              type="button"
              className="selection-action-btn"
              onClick={() =>
                void onTranslate(
                  untranslatedRecordings.map((recording) => recording.filePath),
                )
              }
              disabled={busyAction === "translateRecording"}
            >
              Translate All
            </button>
          ) : null}
          {allowMp3Conversion && convertibleRecordings.length > 0 ? (
            <TooltipWrap description={MP3_CONVERSION_WARNING}>
              <button
                type="button"
                className="selection-action-btn"
                onClick={() =>
                  void onConvertToMp3(
                    convertibleRecordings.map((recording) => recording.filePath),
                  )
                }
                disabled={busyAction === "convertMp3"}
              >
                Convert All WAV
              </button>
            </TooltipWrap>
          ) : null}
        </div>

        {/* RIGHT: a quiet Anki cluster — the default-deck selector paired with a
            ghost Refresh. Grouped so they read as one unit. */}
        <div className="recording-toolbar-anki">
          <DropdownMenuPrimitive.Root>
            <DropdownMenuPrimitive.Trigger asChild>
              <button
                type="button"
                className="deck-select-trigger"
                disabled={configuredDeckMenuOptions.length === 0}
                title="Change the default Anki deck for normal pushes"
              >
                <span className="deck-select-label">
                  Deck: {configuredAnkiDeckLabel}
                </span>
                <span className="deck-select-caret" aria-hidden="true">
                  {"▾"}
                </span>
              </button>
            </DropdownMenuPrimitive.Trigger>
            <DropdownMenuPrimitive.Portal>
              <DropdownMenuPrimitive.Content
                className="action-menu-content target-deck-menu-content"
                align="start"
                sideOffset={6}
              >
                <DropdownMenuPrimitive.Label className="action-menu-label">
                  Default push deck
                </DropdownMenuPrimitive.Label>
                {configuredDeckMenuOptions.map((deck) => (
                  <DropdownMenuPrimitive.Item
                    key={deck}
                    className="action-menu-item"
                    onSelect={() => onDefaultDeckChange(deck)}
                  >
                    <span>{deck}</span>
                    {currentDeckName === deck ? (
                      <span className="action-menu-meta">Current</span>
                    ) : null}
                  </DropdownMenuPrimitive.Item>
                ))}
              </DropdownMenuPrimitive.Content>
            </DropdownMenuPrimitive.Portal>
          </DropdownMenuPrimitive.Root>
          <button
            type="button"
            className="ghost"
            onClick={() => void onRefreshAnki()}
            disabled={busyAction === "loadAnki"}
            title="Refresh Anki decks, note types, fields, and pushed-card status"
          >
            <span aria-hidden="true">{"↻"}</span> Refresh
          </button>
        </div>
      </div>
    </>
  );
}
