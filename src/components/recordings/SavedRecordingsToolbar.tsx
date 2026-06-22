import * as DropdownMenuPrimitive from "@radix-ui/react-dropdown-menu";
import { MP3_CONVERSION_WARNING } from "../../constants";
import type { RecordingFilterTab } from "../../lib/navigation";
import type { BusyAction, RecentRecording, RecordingFilter } from "../../types";
import { TooltipWrap } from "../ui/Tooltip";
import { RecordingFilterTabs } from "./RecordingFilterTabs";

type RecordingAction = (filePaths: string[]) => void | Promise<void>;
type PushAction = (filePaths: string[], deckName?: string) => void | Promise<void>;

export function SavedRecordingsToolbar({
  recordingFilter,
  recordingFilterTabs,
  visibleRecordingsCount,
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
  untranscribedRecordings,
  pushableRecordings,
  untranslatedRecordings,
  convertibleRecordings,
  selectedRecordingsPushableToDeck,
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
  visibleRecordingsCount: number;
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
  untranscribedRecordings: RecentRecording[];
  pushableRecordings: RecentRecording[];
  untranslatedRecordings: RecentRecording[];
  convertibleRecordings: RecentRecording[];
  selectedRecordingsPushableToDeck: (deckName: string) => RecentRecording[];
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

  return (
    <>
      <RecordingFilterTabs
        value={recordingFilter}
        tabs={recordingFilterTabs}
        onChange={onFilterChange}
      />

      <div
        className={`recording-toolbar ${
          hasSelection ? "recording-toolbar-selected" : ""
        }`}
      >
        <div className="recording-toolbar-summary">
          <span className="selection-summary">
            {hasSelection
              ? `${visibleSelectedPaths.length} selected`
              : `${visibleRecordingsCount} shown`}
          </span>
          <DropdownMenuPrimitive.Root>
            <DropdownMenuPrimitive.Trigger asChild>
              <button
                type="button"
                className="target-deck-pill target-deck-trigger"
                disabled={configuredDeckMenuOptions.length === 0}
                title="Change the default Anki deck for normal pushes"
              >
                Deck: {configuredAnkiDeckLabel}
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
            className="target-deck-pill target-deck-trigger"
            onClick={() => void onRefreshAnki()}
            disabled={busyAction === "loadAnki"}
            title="Refresh Anki decks, note types, fields, and pushed-card status"
          >
            Refresh Anki
          </button>
        </div>
        <div className="recording-toolbar-actions">
          {hasSelection ? (
            <DropdownMenuPrimitive.Root>
              <DropdownMenuPrimitive.Trigger asChild>
                <button
                  type="button"
                  className="secondary selected-actions-trigger"
                >
                  Batch Actions
                </button>
              </DropdownMenuPrimitive.Trigger>
              <DropdownMenuPrimitive.Portal>
                <DropdownMenuPrimitive.Content
                  className="action-menu-content selected-actions-menu-content"
                  align="end"
                  sideOffset={6}
                >
                  <DropdownMenuPrimitive.Label className="action-menu-label">
                    Selected recordings
                  </DropdownMenuPrimitive.Label>
                  {selectedUntranscribedRecordings.length > 0 ? (
                    <DropdownMenuPrimitive.Item
                      className="action-menu-item"
                      onSelect={() =>
                        void onTranscribe(
                          selectedUntranscribedRecordings.map(
                            (recording) => recording.filePath,
                          ),
                        )
                      }
                      disabled={busyAction === "transcribeRecording"}
                    >
                      Transcribe
                      <span className="action-menu-meta">
                        {selectedUntranscribedRecordings.length}
                      </span>
                    </DropdownMenuPrimitive.Item>
                  ) : null}
                  {selectedPushableRecordings.length > 0 ? (
                    <DropdownMenuPrimitive.Item
                      className="action-menu-item"
                      onSelect={() =>
                        void onPushToAnki(
                          selectedPushableRecordings.map(
                            (recording) => recording.filePath,
                          ),
                        )
                      }
                      disabled={busyAction === "pushAnki"}
                    >
                      Push to target deck
                      <span className="action-menu-meta">
                        {selectedPushableRecordings.length}
                      </span>
                    </DropdownMenuPrimitive.Item>
                  ) : null}
                  {selectedTranscribedRecordings.some(
                    (recording) => !recording.audioDeleted,
                  ) && configuredDeckMenuOptions.length > 0 ? (
                    <DropdownMenuPrimitive.Sub>
                      <DropdownMenuPrimitive.SubTrigger
                        className="action-menu-item action-menu-sub-trigger"
                        disabled={busyAction === "pushAnki"}
                      >
                        Push to another deck
                        <span className="action-menu-sub-arrow" aria-hidden="true">
                          &gt;
                        </span>
                      </DropdownMenuPrimitive.SubTrigger>
                      <DropdownMenuPrimitive.Portal>
                        <DropdownMenuPrimitive.SubContent
                          className="action-menu-content action-menu-sub-content"
                          sideOffset={8}
                          alignOffset={-4}
                        >
                          <DropdownMenuPrimitive.Label className="action-menu-label">
                            Choose target deck
                          </DropdownMenuPrimitive.Label>
                          {configuredDeckMenuOptions.map((deck) => {
                            const deckPushable =
                              selectedRecordingsPushableToDeck(deck);
                            return (
                              <DropdownMenuPrimitive.Item
                                key={deck}
                                className="action-menu-item"
                                onSelect={() =>
                                  void onPushToAnki(
                                    deckPushable.map(
                                      (recording) => recording.filePath,
                                    ),
                                    deck,
                                  )
                                }
                                disabled={
                                  deckPushable.length === 0 ||
                                  busyAction === "pushAnki"
                                }
                              >
                                <span>{deck}</span>
                                <span className="action-menu-meta">
                                  {deckPushable.length > 0
                                    ? deckPushable.length
                                    : "Done"}
                                </span>
                              </DropdownMenuPrimitive.Item>
                            );
                          })}
                        </DropdownMenuPrimitive.SubContent>
                      </DropdownMenuPrimitive.Portal>
                    </DropdownMenuPrimitive.Sub>
                  ) : null}
                  {selectedFuriganaRecordings.length > 0 ? (
                    <DropdownMenuPrimitive.Item
                      className="action-menu-item"
                      onSelect={() =>
                        void onAddFurigana(
                          selectedFuriganaRecordings.map(
                            (recording) => recording.filePath,
                          ),
                        )
                      }
                      disabled={!expressionFieldMapped || busyAction === "addFurigana"}
                    >
                      Add furigana
                      <span className="action-menu-meta">
                        {expressionFieldMapped
                          ? selectedFuriganaRecordings.length
                          : "Map field"}
                      </span>
                    </DropdownMenuPrimitive.Item>
                  ) : null}
                  {selectedUntranslatedRecordings.length > 0 ? (
                    <DropdownMenuPrimitive.Item
                      className="action-menu-item"
                      onSelect={() =>
                        void onTranslate(
                          selectedUntranslatedRecordings.map(
                            (recording) => recording.filePath,
                          ),
                        )
                      }
                      disabled={busyAction === "translateRecording"}
                    >
                      Translate
                      <span className="action-menu-meta">
                        {selectedUntranslatedRecordings.length}
                      </span>
                    </DropdownMenuPrimitive.Item>
                  ) : null}
                  {allowMp3Conversion && selectedConvertibleRecordings.length > 0 ? (
                    <DropdownMenuPrimitive.Item
                      className="action-menu-item"
                      title={MP3_CONVERSION_WARNING}
                      onSelect={() =>
                        void onConvertToMp3(
                          selectedConvertibleRecordings.map(
                            (recording) => recording.filePath,
                          ),
                        )
                      }
                      disabled={busyAction === "convertMp3"}
                    >
                      Convert to MP3
                      <span className="action-menu-meta">
                        {selectedConvertibleRecordings.length}
                      </span>
                    </DropdownMenuPrimitive.Item>
                  ) : null}
                  <DropdownMenuPrimitive.Separator className="action-menu-separator" />
                  <DropdownMenuPrimitive.Item
                    className="action-menu-item action-menu-item-danger"
                    onSelect={() => void onDelete(visibleSelectedPaths)}
                    disabled={busyAction === "deleteRecording"}
                  >
                    Delete
                    <span className="action-menu-meta">
                      {visibleSelectedPaths.length}
                    </span>
                  </DropdownMenuPrimitive.Item>
                </DropdownMenuPrimitive.Content>
              </DropdownMenuPrimitive.Portal>
            </DropdownMenuPrimitive.Root>
          ) : null}
          {!hasSelection &&
          recordingFilter === "needsTranscription" &&
          untranscribedRecordings.length > 0 ? (
            <button
              type="button"
              className="secondary"
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
          {!hasSelection &&
          recordingFilter === "needsAnki" &&
          pushableRecordings.length > 0 ? (
            <button
              type="button"
              className="secondary"
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
          {!hasSelection &&
          recordingFilter === "needsTranslation" &&
          untranslatedRecordings.length > 0 ? (
            <button
              type="button"
              className="secondary"
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
          {!hasSelection && allowMp3Conversion && convertibleRecordings.length > 0 ? (
            <TooltipWrap description={MP3_CONVERSION_WARNING}>
              <button
                type="button"
                className="secondary"
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
          {hasSelection ? (
            <button type="button" className="ghost" onClick={onClearSelection}>
              Clear selection
            </button>
          ) : null}
        </div>
      </div>
    </>
  );
}
