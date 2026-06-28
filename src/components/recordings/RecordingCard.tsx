import * as DropdownMenuPrimitive from "@radix-ui/react-dropdown-menu";
import { MP3_CONVERSION_WARNING } from "../../constants";
import {
  formatBytes,
  formatDuration,
  formatTimestamp,
} from "../../lib/format";
import {
  pathHasExtension,
  recordingSupportsFurigana,
  transcriptLanguageLabel,
} from "../../lib/helpers";
import type { BusyAction, RecentRecording } from "../../types";
import { TooltipWrap } from "../ui/Tooltip";

type RecordingAction = (filePaths: string[]) => void | Promise<void>;
type SingleRecordingAction = (filePath: string) => void | Promise<void>;
type PushAction = (filePaths: string[], deckName?: string) => void | Promise<void>;

export function RecordingCard({
  recording,
  selected,
  useBatchActionsOnly,
  open,
  busyAction,
  configuredAnkiDeckLabel,
  configuredDeckName,
  availableAnkiDecks,
  allowMp3Conversion,
  expressionFieldMapped,
  recordingPushedToDeck,
  recordingPushedToCurrentAnkiDeck,
  onToggleSelection,
  onOpenChange,
  onPlay,
  onTranscribe,
  onPushToAnki,
  onAddFurigana,
  onTranslate,
  onConvertToMp3,
  onDelete,
}: {
  recording: RecentRecording;
  selected: boolean;
  useBatchActionsOnly: boolean;
  open: boolean;
  busyAction: BusyAction;
  configuredAnkiDeckLabel: string;
  configuredDeckName: string;
  availableAnkiDecks: string[];
  allowMp3Conversion: boolean;
  expressionFieldMapped: boolean;
  recordingPushedToDeck: (recording: RecentRecording, deckName: string) => boolean;
  recordingPushedToCurrentAnkiDeck: (recording: RecentRecording) => boolean;
  onToggleSelection: (filePath: string) => void;
  onOpenChange: (filePath: string | null) => void;
  onPlay: SingleRecordingAction;
  onTranscribe: RecordingAction;
  onPushToAnki: PushAction;
  onAddFurigana: RecordingAction;
  onTranslate: RecordingAction;
  onConvertToMp3: RecordingAction;
  onDelete: SingleRecordingAction;
}) {
  const canPushToConfiguredDeck =
    Boolean(recording.transcriptPath) &&
    !recording.audioDeleted &&
    !recordingPushedToCurrentAnkiDeck(recording);
  const canPushToAnyDeck =
    Boolean(recording.transcriptPath) && !recording.audioDeleted;
  const canAddFuriganaToCard =
    Boolean(recording.transcriptPath) &&
    recording.ankiNoteId !== null &&
    !recording.furiganaApplied &&
    recordingSupportsFurigana(recording);
  const languageLabel = transcriptLanguageLabel(recording.transcriptLanguage);

  return (
    <article className="recording-item">
      <div className="recording-head">
        <label className="recording-select">
          <input
            type="checkbox"
            checked={selected}
            onChange={() => onToggleSelection(recording.filePath)}
            aria-label={`Select ${recording.fileName}`}
          />
          <strong>{recording.fileName}</strong>
        </label>
        <span>{formatDuration(recording.durationMs)}</span>
      </div>
      <div className="recording-meta">
        <span>{formatBytes(recording.bytesWritten)}</span>
        <span>{formatTimestamp(recording.createdAtMs)}</span>
      </div>
      <div
        className="recording-state-row"
        title={
          recording.transcriptPath
            ? `Audio: ${recording.filePath}\nTranscript: ${recording.transcriptPath}`
            : `Audio: ${recording.filePath}`
        }
      >
        <span className="recording-state">
          {recording.audioDeleted
            ? "Transcript only - local audio deleted"
            : recording.transcriptPath
              ? "Audio + transcript"
              : "Audio only"}
        </span>
        {recording.ankiNoteId !== null ? (
          <span
            className="recording-state success-state"
            title={
              recording.ankiDeckName
                ? `Pushed to ${recording.ankiDeckName}${
                    recording.ankiNoteType ? ` / ${recording.ankiNoteType}` : ""
                  }`
                : "Pushed to Anki"
            }
          >
            {recording.ankiDeckName ? `Anki: ${recording.ankiDeckName}` : "In Anki"}
          </span>
        ) : null}
        {recording.translationPath !== null ? (
          <span className="recording-state success-state">Translated</span>
        ) : null}
        {languageLabel ? (
          <span className="recording-state">{languageLabel}</span>
        ) : null}
      </div>
      <div className="recording-actions">
        <DropdownMenuPrimitive.Root
          modal={false}
          open={!useBatchActionsOnly && open}
          onOpenChange={(nextOpen) =>
            onOpenChange(nextOpen && !useBatchActionsOnly ? recording.filePath : null)
          }
        >
          <DropdownMenuPrimitive.Trigger asChild>
            <button
              type="button"
              className="secondary compact-menu-trigger"
              disabled={useBatchActionsOnly}
              title={
                useBatchActionsOnly
                  ? "Use Batch Actions while multiple recordings are selected."
                  : "Open recording actions"
              }
            >
              Actions
            </button>
          </DropdownMenuPrimitive.Trigger>
          <DropdownMenuPrimitive.Portal>
            <DropdownMenuPrimitive.Content
              className="action-menu-content"
              align="end"
              sideOffset={6}
            >
              <DropdownMenuPrimitive.Label className="action-menu-label">
                Actions
              </DropdownMenuPrimitive.Label>
              <DropdownMenuPrimitive.Item
                className="action-menu-item"
                onSelect={() => void onPlay(recording.filePath)}
                disabled={recording.audioDeleted || busyAction === "playRecording"}
              >
                Play
              </DropdownMenuPrimitive.Item>
              {!recording.transcriptPath ? (
                <DropdownMenuPrimitive.Item
                  className="action-menu-item"
                  onSelect={() => void onTranscribe([recording.filePath])}
                  disabled={busyAction === "transcribeRecording"}
                >
                  Transcribe
                </DropdownMenuPrimitive.Item>
              ) : null}
              {recording.transcriptPath ? (
                <>
                  <DropdownMenuPrimitive.Separator className="action-menu-separator" />
                  <DropdownMenuPrimitive.Item
                    className="action-menu-item"
                    onSelect={() => void onPushToAnki([recording.filePath])}
                    disabled={
                      !canPushToConfiguredDeck ||
                      !configuredDeckName ||
                      busyAction === "pushAnki"
                    }
                  >
                    Push to {configuredAnkiDeckLabel}
                  </DropdownMenuPrimitive.Item>
                  <DropdownMenuPrimitive.Sub>
                    <DropdownMenuPrimitive.SubTrigger
                      className="action-menu-item action-menu-sub-trigger"
                      disabled={
                        !canPushToAnyDeck ||
                        availableAnkiDecks.length === 0 ||
                        busyAction === "pushAnki"
                      }
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
                        {availableAnkiDecks.map((deck) => {
                          const alreadyPushedToDeck = recordingPushedToDeck(
                            recording,
                            deck,
                          );
                          return (
                            <DropdownMenuPrimitive.Item
                              key={deck}
                              className="action-menu-item"
                              onSelect={() => {
                                onOpenChange(null);
                                void onPushToAnki([recording.filePath], deck);
                              }}
                              disabled={
                                alreadyPushedToDeck || busyAction === "pushAnki"
                              }
                            >
                              <span>{deck}</span>
                              {alreadyPushedToDeck ? (
                                <span className="action-menu-meta">Done</span>
                              ) : null}
                            </DropdownMenuPrimitive.Item>
                          );
                        })}
                      </DropdownMenuPrimitive.SubContent>
                    </DropdownMenuPrimitive.Portal>
                  </DropdownMenuPrimitive.Sub>
                  {recordingSupportsFurigana(recording) &&
                  !recording.furiganaApplied ? (
                    <DropdownMenuPrimitive.Item
                      className="action-menu-item"
                      onSelect={() => void onAddFurigana([recording.filePath])}
                      disabled={
                        !canAddFuriganaToCard ||
                        !expressionFieldMapped ||
                        busyAction === "addFurigana"
                      }
                    >
                      Add furigana
                      {!expressionFieldMapped ? (
                        <span className="action-menu-meta">Map field</span>
                      ) : null}
                    </DropdownMenuPrimitive.Item>
                  ) : null}
                </>
              ) : null}
              {recording.transcriptPath && recording.translationPath === null ? (
                <DropdownMenuPrimitive.Item
                  className="action-menu-item"
                  onSelect={() => void onTranslate([recording.filePath])}
                  disabled={busyAction === "translateRecording"}
                >
                  Translate
                </DropdownMenuPrimitive.Item>
              ) : null}
              {allowMp3Conversion &&
              recording.transcriptPath &&
              !recording.audioDeleted &&
              pathHasExtension(recording.filePath, "wav") ? (
                <TooltipWrap description={MP3_CONVERSION_WARNING}>
                  <DropdownMenuPrimitive.Item
                    className="action-menu-item"
                    onSelect={() => void onConvertToMp3([recording.filePath])}
                    disabled={busyAction === "convertMp3"}
                  >
                    Convert to MP3
                  </DropdownMenuPrimitive.Item>
                </TooltipWrap>
              ) : null}
              <DropdownMenuPrimitive.Separator className="action-menu-separator" />
              <DropdownMenuPrimitive.Item
                className="action-menu-item action-menu-item-danger"
                onSelect={() => void onDelete(recording.filePath)}
                disabled={busyAction === "deleteRecording"}
              >
                Delete
              </DropdownMenuPrimitive.Item>
            </DropdownMenuPrimitive.Content>
          </DropdownMenuPrimitive.Portal>
        </DropdownMenuPrimitive.Root>
      </div>
    </article>
  );
}
