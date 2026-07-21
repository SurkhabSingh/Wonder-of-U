import * as DropdownMenuPrimitive from "@radix-ui/react-dropdown-menu";
import { MP3_CONVERSION_WARNING } from "../../constants";
import type { BusyAction, RecentRecording } from "../../types";

type RecordingAction = (filePaths: string[]) => void | Promise<void>;
type PushAction = (filePaths: string[], deckName?: string) => void | Promise<void>;

// Select-mode content for the Library toolbar. When rows are selected the
// filter tabs + normal toolbar give way to this row IN PLACE (Gmail-style takeover)
// — nothing new floats or docks. It reuses the .recording-toolbar row shell, adds
// the .recording-toolbar-selected tint + an accent left-rail, and surfaces the
// applicable actions DIRECTLY as one-click buttons (each with its own count),
// with a small dropdown only for the rarer "push to another deck".
export function SelectionActionBar({
  visibleSelectedPaths,
  configuredDeckMenuOptions,
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
  onTranscribe,
  onPushToAnki,
  onAddFurigana,
  onTranslate,
  onConvertToMp3,
  onDelete,
  onClearSelection,
}: {
  visibleSelectedPaths: string[];
  configuredDeckMenuOptions: string[];
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
  onTranscribe: RecordingAction;
  onPushToAnki: PushAction;
  onAddFurigana: RecordingAction;
  onTranslate: RecordingAction;
  onConvertToMp3: RecordingAction;
  onDelete: RecordingAction;
  onClearSelection: () => void;
}) {
  const paths = (recordings: RecentRecording[]) =>
    recordings.map((recording) => recording.filePath);

  const canPushToAnotherDeck =
    configuredDeckMenuOptions.length > 0 &&
    selectedTranscribedRecordings.some((recording) => !recording.audioDeleted);

  return (
    <div
      className="recording-toolbar recording-toolbar-selected recording-toolbar-select-mode"
      role="region"
      aria-label="Selection actions"
    >
      <span className="selection-action-count">
        {visibleSelectedPaths.length} selected
      </span>

      <div className="selection-action-buttons">
        {selectedUntranscribedRecordings.length > 0 ? (
          <ActionButton
            label="Transcribe"
            count={selectedUntranscribedRecordings.length}
            onClick={() => void onTranscribe(paths(selectedUntranscribedRecordings))}
          />
        ) : null}
        {selectedUntranslatedRecordings.length > 0 ? (
          <ActionButton
            label="Translate"
            count={selectedUntranslatedRecordings.length}
            disabled={busyAction === "translateRecording"}
            onClick={() => void onTranslate(paths(selectedUntranslatedRecordings))}
          />
        ) : null}
        {selectedPushableRecordings.length > 0 ? (
          <ActionButton
            label="Push to Anki"
            count={selectedPushableRecordings.length}
            disabled={busyAction === "pushAnki"}
            onClick={() => void onPushToAnki(paths(selectedPushableRecordings))}
          />
        ) : null}
        {selectedFuriganaRecordings.length > 0 ? (
          <ActionButton
            label="Furigana"
            count={
              expressionFieldMapped
                ? selectedFuriganaRecordings.length
                : undefined
            }
            hint={expressionFieldMapped ? undefined : "Map field"}
            disabled={!expressionFieldMapped || busyAction === "addFurigana"}
            onClick={() => void onAddFurigana(paths(selectedFuriganaRecordings))}
          />
        ) : null}
        {allowMp3Conversion && selectedConvertibleRecordings.length > 0 ? (
          <ActionButton
            label="Convert to MP3"
            count={selectedConvertibleRecordings.length}
            title={MP3_CONVERSION_WARNING}
            disabled={busyAction === "convertMp3"}
            onClick={() => void onConvertToMp3(paths(selectedConvertibleRecordings))}
          />
        ) : null}
        {canPushToAnotherDeck ? (
          <DropdownMenuPrimitive.Root>
            <DropdownMenuPrimitive.Trigger asChild>
              <button
                type="button"
                className="selection-action-btn"
                disabled={busyAction === "pushAnki"}
              >
                Push to deck…
              </button>
            </DropdownMenuPrimitive.Trigger>
            <DropdownMenuPrimitive.Portal>
              <DropdownMenuPrimitive.Content
                className="action-menu-content"
                align="end"
                side="top"
                sideOffset={6}
              >
                <DropdownMenuPrimitive.Label className="action-menu-label">
                  Push selected to
                </DropdownMenuPrimitive.Label>
                {configuredDeckMenuOptions.map((deck) => {
                  const deckPushable = selectedRecordingsPushableToDeck(deck);
                  return (
                    <DropdownMenuPrimitive.Item
                      key={deck}
                      className="action-menu-item"
                      onSelect={() => void onPushToAnki(paths(deckPushable), deck)}
                      disabled={
                        deckPushable.length === 0 || busyAction === "pushAnki"
                      }
                    >
                      <span>{deck}</span>
                      <span className="action-menu-meta">
                        {deckPushable.length > 0 ? deckPushable.length : "Done"}
                      </span>
                    </DropdownMenuPrimitive.Item>
                  );
                })}
              </DropdownMenuPrimitive.Content>
            </DropdownMenuPrimitive.Portal>
          </DropdownMenuPrimitive.Root>
        ) : null}
      </div>

      <div className="selection-action-end">
        <ActionButton
          label="Delete"
          count={visibleSelectedPaths.length}
          danger
          disabled={busyAction === "deleteRecording"}
          onClick={() => void onDelete(visibleSelectedPaths)}
        />
        <button
          type="button"
          className="ghost selection-action-clear"
          onClick={onClearSelection}
        >
          Clear
        </button>
      </div>
    </div>
  );
}

function ActionButton({
  label,
  count,
  hint,
  title,
  danger = false,
  disabled = false,
  onClick,
}: {
  label: string;
  count?: number;
  hint?: string;
  title?: string;
  danger?: boolean;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className={`selection-action-btn ${danger ? "is-danger" : ""}`}
      title={title}
      disabled={disabled}
      onClick={onClick}
    >
      {label}
      {count !== undefined ? (
        <span className="selection-action-badge">{count}</span>
      ) : null}
      {hint !== undefined ? (
        <span className="selection-action-badge is-hint">{hint}</span>
      ) : null}
    </button>
  );
}
