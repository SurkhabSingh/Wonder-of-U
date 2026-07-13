import type { RecordingFilterTab } from "../../lib/navigation";
import type { BusyAction, RecentRecording, RecordingFilter } from "../../types";
import { RecordingCard } from "./RecordingCard";
import { SavedRecordingsToolbar } from "./SavedRecordingsToolbar";

type RecordingAction = (filePaths: string[]) => void | Promise<void>;
type SingleRecordingAction = (filePath: string) => void | Promise<void>;
type PushAction = (filePaths: string[], deckName?: string) => void | Promise<void>;

export function SavedRecordingsPage({
  recordingActionMessage,
  recentRecordings,
  visibleRecordings,
  recordingFilter,
  recordingFilterTabs,
  recordingPage,
  recordingPageCount,
  recordingPageStart,
  recordingPageEnd,
  filteredRecordingsCount,
  selectedRecordings,
  visibleSelectedPaths,
  configuredAnkiDeckLabel,
  configuredDeckMenuOptions,
  currentDeckName,
  currentNoteType,
  availableAnkiDecks,
  transcriptionLanguage,
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
  openRecordingMenuPath,
  selectedRecordingsPushableToDeck,
  recordingPushedToDeck,
  recordingPushedToCurrentAnkiDeck,
  onFilterChange,
  onPageChange,
  onDefaultDeckChange,
  onRefreshAnki,
  onToggleSelection,
  onClearSelection,
  onOpenRecordingMenuChange,
  onPlay,
  onTranscribe,
  onPushToAnki,
  onAddFurigana,
  onTranslate,
  onConvertToMp3,
  onDeleteRecording,
  onDeleteRecordings,
  onView,
}: {
  recordingActionMessage: string;
  recentRecordings: RecentRecording[];
  visibleRecordings: RecentRecording[];
  recordingFilter: RecordingFilter;
  recordingFilterTabs: RecordingFilterTab[];
  recordingPage: number;
  recordingPageCount: number;
  recordingPageStart: number;
  recordingPageEnd: number;
  filteredRecordingsCount: number;
  selectedRecordings: string[];
  visibleSelectedPaths: string[];
  configuredAnkiDeckLabel: string;
  configuredDeckMenuOptions: string[];
  currentDeckName: string;
  currentNoteType: string;
  availableAnkiDecks: string[];
  transcriptionLanguage: string;
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
  openRecordingMenuPath: string | null;
  selectedRecordingsPushableToDeck: (deckName: string) => RecentRecording[];
  recordingPushedToDeck: (recording: RecentRecording, deckName: string) => boolean;
  recordingPushedToCurrentAnkiDeck: (recording: RecentRecording) => boolean;
  onFilterChange: (filter: RecordingFilter) => void;
  onPageChange: (page: number) => void;
  onDefaultDeckChange: (deckName: string) => void;
  onRefreshAnki: () => void | Promise<void>;
  onToggleSelection: (filePath: string) => void;
  onClearSelection: () => void;
  onOpenRecordingMenuChange: (filePath: string | null) => void;
  onPlay: SingleRecordingAction;
  onTranscribe: RecordingAction;
  onPushToAnki: PushAction;
  onAddFurigana: RecordingAction;
  onTranslate: RecordingAction;
  onConvertToMp3: RecordingAction;
  onDeleteRecording: SingleRecordingAction;
  onDeleteRecordings: RecordingAction;
  onView: SingleRecordingAction;
}) {
  const selectedRecordingSet = new Set(selectedRecordings);
  const useBatchActionsOnly = visibleSelectedPaths.length > 1;

  return (
    <div className="recorder-view recordings-view">
      <article className="panel recent-panel">
        <header className="panel-header">
          <div>
            <p className="panel-kicker">Recent Output</p>
            <h2>Saved Recordings</h2>
          </div>
        </header>

        {recordingActionMessage ? (
          <p className="microcopy">{recordingActionMessage}</p>
        ) : null}

        {recentRecordings.length > 0 ? (
          <SavedRecordingsToolbar
            recordingFilter={recordingFilter}
            recordingFilterTabs={recordingFilterTabs}
            visibleRecordingsCount={filteredRecordingsCount}
            visibleSelectedPaths={visibleSelectedPaths}
            configuredAnkiDeckLabel={configuredAnkiDeckLabel}
            configuredDeckMenuOptions={configuredDeckMenuOptions}
            currentDeckName={currentDeckName}
            busyAction={busyAction}
            allowMp3Conversion={allowMp3Conversion}
            expressionFieldMapped={expressionFieldMapped}
            selectedUntranscribedRecordings={selectedUntranscribedRecordings}
            selectedPushableRecordings={selectedPushableRecordings}
            selectedTranscribedRecordings={selectedTranscribedRecordings}
            selectedFuriganaRecordings={selectedFuriganaRecordings}
            selectedUntranslatedRecordings={selectedUntranslatedRecordings}
            selectedConvertibleRecordings={selectedConvertibleRecordings}
            untranscribedRecordings={untranscribedRecordings}
            pushableRecordings={pushableRecordings}
            untranslatedRecordings={untranslatedRecordings}
            convertibleRecordings={convertibleRecordings}
            selectedRecordingsPushableToDeck={selectedRecordingsPushableToDeck}
            onFilterChange={onFilterChange}
            onDefaultDeckChange={onDefaultDeckChange}
            onRefreshAnki={onRefreshAnki}
            onTranscribe={onTranscribe}
            onPushToAnki={onPushToAnki}
            onAddFurigana={onAddFurigana}
            onTranslate={onTranslate}
            onConvertToMp3={onConvertToMp3}
            onDelete={onDeleteRecordings}
            onClearSelection={onClearSelection}
          />
        ) : null}

        {recentRecordings.length === 0 ? (
          <p className="empty-state">No recordings yet</p>
        ) : visibleRecordings.length === 0 ? (
          <p className="empty-state">No recordings in this status</p>
        ) : (
          <div className="recording-list">
            {visibleRecordings.map((recording) => (
              <RecordingCard
                key={recording.filePath}
                recording={recording}
                selected={selectedRecordingSet.has(recording.filePath)}
                useBatchActionsOnly={useBatchActionsOnly}
                open={openRecordingMenuPath === recording.filePath}
                busyAction={busyAction}
                configuredAnkiDeckLabel={configuredAnkiDeckLabel}
                configuredDeckName={currentDeckName}
                configuredNoteType={currentNoteType}
                availableAnkiDecks={availableAnkiDecks}
                transcriptionLanguage={transcriptionLanguage}
                allowMp3Conversion={allowMp3Conversion}
                expressionFieldMapped={expressionFieldMapped}
                recordingPushedToDeck={recordingPushedToDeck}
                recordingPushedToCurrentAnkiDeck={recordingPushedToCurrentAnkiDeck}
                onToggleSelection={onToggleSelection}
                onOpenChange={onOpenRecordingMenuChange}
                onPlay={onPlay}
                onTranscribe={onTranscribe}
                onPushToAnki={onPushToAnki}
                onAddFurigana={onAddFurigana}
                onTranslate={onTranslate}
                onConvertToMp3={onConvertToMp3}
                onDelete={onDeleteRecording}
                onView={onView}
              />
            ))}
          </div>
        )}

        {filteredRecordingsCount > 0 ? (
          <nav className="recording-pagination" aria-label="Saved recordings pages">
            <span className="recording-page-summary">
              {recordingPageStart}-{recordingPageEnd} of {filteredRecordingsCount}
            </span>
            <div className="recording-page-actions">
              <button
                type="button"
                className="secondary compact"
                disabled={recordingPage <= 1}
                onClick={() => onPageChange(recordingPage - 1)}
              >
                Previous
              </button>
              <span className="recording-page-number">
                Page {recordingPage} of {recordingPageCount}
              </span>
              <button
                type="button"
                className="secondary compact"
                disabled={recordingPage >= recordingPageCount}
                onClick={() => onPageChange(recordingPage + 1)}
              >
                Next
              </button>
            </div>
          </nav>
        ) : null}
      </article>
    </div>
  );
}
