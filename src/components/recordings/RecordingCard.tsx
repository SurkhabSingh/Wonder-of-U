import * as DropdownMenuPrimitive from "@radix-ui/react-dropdown-menu";
import { MP3_CONVERSION_WARNING } from "../../constants";
import {
  formatBytes,
  formatDuration,
  formatTimestamp,
} from "../../lib/format";
import {
  pathHasExtension,
  recordingAnkiPushForTarget,
  recordingChips,
  recordingHasTranscriptForLanguage,
  recordingSupportsFurigana,
  recordingTranscriptLanguageLabels,
  transcriptLanguageLabel,
} from "../../lib/helpers";
import type { BusyAction, RecentRecording } from "../../types";
import { TooltipWrap } from "../ui/Tooltip";

type RecordingAction = (filePaths: string[]) => void | Promise<void>;
type SingleRecordingAction = (filePath: string) => void | Promise<void>;
type PushAction = (filePaths: string[], deckName?: string) => void | Promise<void>;
// `force` re-runs translation on a recording that already has one.
type TranslateAction = (
  filePaths: string[],
  force?: boolean,
) => void | Promise<void>;

export function RecordingCard({
  recording,
  selected,
  useBatchActionsOnly,
  open,
  busyAction,
  configuredAnkiDeckLabel,
  configuredDeckName,
  configuredNoteType,
  availableAnkiDecks,
  transcriptionLanguage,
  allowMp3Conversion,
  expressionFieldMapped,
  recordingPushedToDeck,
  recordingPushedToCurrentAnkiDeck,
  onToggleSelection,
  onOpenChange,
  onPlay,
  onTranscribe,
  onReTranscribe,
  onPushToAnki,
  onAddFurigana,
  onTranslate,
  onConvertToMp3,
  onDelete,
  onView,
}: {
  recording: RecentRecording;
  selected: boolean;
  useBatchActionsOnly: boolean;
  open: boolean;
  busyAction: BusyAction;
  configuredAnkiDeckLabel: string;
  configuredDeckName: string;
  configuredNoteType: string;
  availableAnkiDecks: string[];
  transcriptionLanguage: string;
  allowMp3Conversion: boolean;
  expressionFieldMapped: boolean;
  recordingPushedToDeck: (recording: RecentRecording, deckName: string) => boolean;
  recordingPushedToCurrentAnkiDeck: (recording: RecentRecording) => boolean;
  onToggleSelection: (filePath: string) => void;
  onOpenChange: (filePath: string | null) => void;
  onPlay: SingleRecordingAction;
  onTranscribe: RecordingAction;
  // Force re-runs transcription on a recording that already has one (e.g. to redo it
  // after switching Audio type to Music, or changing the model/language).
  onReTranscribe: RecordingAction;
  onPushToAnki: PushAction;
  onAddFurigana: RecordingAction;
  onTranslate: TranslateAction;
  onConvertToMp3: RecordingAction;
  onDelete: SingleRecordingAction;
  onView: SingleRecordingAction;
}) {
  const hasSelectedTranscript = recordingHasTranscriptForLanguage(
    recording,
    transcriptionLanguage,
  );
  const selectedAnkiPush = recordingAnkiPushForTarget(
    recording,
    transcriptionLanguage,
    configuredDeckName,
    configuredNoteType,
  );
  const canPushToConfiguredDeck =
    hasSelectedTranscript &&
    !recording.audioDeleted &&
    !recordingPushedToCurrentAnkiDeck(recording);
  const canPushToAnyDeck =
    hasSelectedTranscript && !recording.audioDeleted;
  const canAddFuriganaToCard =
    hasSelectedTranscript &&
    selectedAnkiPush !== null &&
    !selectedAnkiPush.furiganaApplied &&
    recordingSupportsFurigana(recording, transcriptionLanguage);
  const languageLabels = recordingTranscriptLanguageLabels(recording);
  const selectedLanguageLabel =
    transcriptLanguageLabel(transcriptionLanguage) ??
    transcriptionLanguage.toUpperCase();
  const ankiPushSummary = recording.ankiPushes
    .map((push) => {
      const language =
        transcriptLanguageLabel(push.language) ?? push.language.toUpperCase();
      return `${language}: ${push.deckName} / ${push.noteType}`;
    })
    .join("\n");
  const hasAnyAnkiPush =
    recording.ankiPushes.length > 0 || recording.ankiNoteId !== null;
  const canReadTranscript =
    recording.transcripts.length > 0 || recording.transcriptPath !== null;
  const stateChips = recordingChips(
    recording,
    transcriptionLanguage,
    recordingPushedToCurrentAnkiDeck,
  );

  // The full provenance — paths, the deleted-audio explanation, and every
  // transcribed language — lives in the row's hover title so the visible chip
  // row can stay to a single line of the most-relevant state.
  const stateRowTitleParts: string[] = [];
  if (recording.audioDeleted) {
    stateRowTitleParts.push("Transcript only — local audio deleted");
    if (recording.transcriptPath) {
      stateRowTitleParts.push(`Transcript: ${recording.transcriptPath}`);
    }
  } else {
    stateRowTitleParts.push(`Audio: ${recording.filePath}`);
    if (recording.transcriptPath) {
      stateRowTitleParts.push(`Transcript: ${recording.transcriptPath}`);
    }
  }
  if (languageLabels.length > 0) {
    stateRowTitleParts.push(
      `Transcribed languages: ${languageLabels.join(", ")}`,
    );
  }
  const stateRowTitle = stateRowTitleParts.join("\n");

  // The single most-relevant next step, surfaced as a one-click primary button.
  // Priority mirrors recordingChips(); each case invokes the same handler its
  // matching overflow-menu item uses, so no new behavior is introduced.
  const primaryAction = !hasSelectedTranscript
    ? {
        label: "Transcribe",
        // Enqueues into the non-blocking transcription queue — stays enabled
        // (re-clicks dedupe on file path), like the YouTube queue's Add.
        onClick: () => void onTranscribe([recording.filePath]),
        disabled: false,
      }
    : recording.transcriptPath && recording.translationPath === null
      ? {
          label: "Translate",
          onClick: () => void onTranslate([recording.filePath]),
          disabled: busyAction === "translateRecording",
        }
      : canPushToConfiguredDeck && configuredDeckName
        ? {
            label: `Push to ${configuredAnkiDeckLabel}`,
            onClick: () => void onPushToAnki([recording.filePath]),
            disabled: busyAction === "pushAnki",
          }
        : canReadTranscript
          ? {
              label: "Read",
              onClick: () => void onView(recording.filePath),
              disabled: false,
            }
          : {
              label: "Play",
              onClick: () => void onPlay(recording.filePath),
              disabled:
                recording.audioDeleted || busyAction === "playRecording",
            };

  return (
    <article className={`recording-item ${selected ? "is-selected" : ""}`}>
      <div className="recording-select">
        <input
          type="checkbox"
          checked={selected}
          onChange={() => onToggleSelection(recording.filePath)}
          aria-label={`Select ${recording.fileName}`}
        />
      </div>
      <div className="recording-main">
        {canReadTranscript ? (
          <button
            type="button"
            className="recording-filename-button"
            onClick={() => void onView(recording.filePath)}
            title="Read transcript and translation"
          >
            {recording.fileName}
          </button>
        ) : (
          <strong className="recording-name">{recording.fileName}</strong>
        )}
        <span className="recording-meta">
          {formatDuration(recording.durationMs)} ·{" "}
          {formatBytes(recording.bytesWritten)} ·{" "}
          {formatTimestamp(recording.createdAtMs)}
        </span>
        <div className="recording-state-row" title={stateRowTitle}>
          {stateChips.map((chip) => (
            <span
              key={chip.label}
              className={`status-chip status-chip-${chip.tone}`}
            >
              {chip.label}
            </span>
          ))}
          {recording.audioDeleted ? (
            <span className="status-chip status-chip-neutral">
              Transcript only
            </span>
          ) : null}
          {hasAnyAnkiPush ? (
            <span
              className="status-chip status-chip-neutral"
              title={
                ankiPushSummary ||
                (recording.ankiDeckName
                  ? `Pushed to ${recording.ankiDeckName}${
                      recording.ankiNoteType ? ` / ${recording.ankiNoteType}` : ""
                    }`
                  : "Pushed to Anki")
              }
            >
              {selectedAnkiPush
                ? `Anki: ${selectedAnkiPush.deckName} (${selectedLanguageLabel})`
                : recording.ankiPushes.length > 0
                  ? `Anki: ${recording.ankiPushes.length} other language ${
                      recording.ankiPushes.length === 1 ? "card" : "cards"
                    }`
                  : recording.ankiDeckName
                    ? `Anki: ${recording.ankiDeckName}`
                    : "In Anki"}
            </span>
          ) : null}
        </div>
      </div>
      <div className="recording-actions">
        <button
          type="button"
          className="secondary recording-primary-action"
          onClick={primaryAction.onClick}
          disabled={useBatchActionsOnly || primaryAction.disabled}
          title={
            useBatchActionsOnly
              ? "Use Batch Actions while multiple recordings are selected."
              : undefined
          }
        >
          {primaryAction.label}
        </button>
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
              className="secondary recording-overflow-trigger"
              disabled={useBatchActionsOnly}
              aria-label="More actions"
              title={
                useBatchActionsOnly
                  ? "Use Batch Actions while multiple recordings are selected."
                  : "More actions"
              }
            >
              <span aria-hidden="true">⋯</span>
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
              {canReadTranscript ? (
                <DropdownMenuPrimitive.Item
                  className="action-menu-item"
                  onSelect={() => void onView(recording.filePath)}
                >
                  Read transcript
                </DropdownMenuPrimitive.Item>
              ) : (
                <TooltipWrap description="Transcribe this recording first to read its transcript.">
                  <span className="action-menu-tooltip-wrap">
                    <DropdownMenuPrimitive.Item
                      className="action-menu-item"
                      disabled
                      onSelect={(event) => event.preventDefault()}
                    >
                      Read transcript
                      <span className="action-menu-meta">No text</span>
                    </DropdownMenuPrimitive.Item>
                  </span>
                </TooltipWrap>
              )}
              {!hasSelectedTranscript ? (
                <DropdownMenuPrimitive.Item
                  className="action-menu-item"
                  onSelect={() => void onTranscribe([recording.filePath])}
                >
                  Transcribe in {selectedLanguageLabel}
                </DropdownMenuPrimitive.Item>
              ) : (
                <DropdownMenuPrimitive.Item
                  className="action-menu-item"
                  onSelect={() => void onReTranscribe([recording.filePath])}
                >
                  Re-transcribe in {selectedLanguageLabel}
                </DropdownMenuPrimitive.Item>
              )}
              {hasSelectedTranscript ? (
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
                        ›
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
                  {recordingSupportsFurigana(recording, transcriptionLanguage) &&
                  !selectedAnkiPush?.furiganaApplied ? (
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
              {recording.transcriptPath && recording.translationPath !== null ? (
                <DropdownMenuPrimitive.Item
                  className="action-menu-item"
                  onSelect={() => void onTranslate([recording.filePath], true)}
                  disabled={busyAction === "translateRecording"}
                >
                  Re-translate
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
