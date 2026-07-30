import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { formatDuration } from "../../lib/format";
import { ThemedSelect } from "../ui/ThemedSelect";
import { SubtitleListPane } from "./SubtitleListPane";
import { ScannableText } from "../scanner/ScannableText";
import { SubtitleOffsetField } from "./SubtitleOffsetField";
import { VideoLibraryList } from "./VideoLibraryList";
import type {
  RecordingSegment,
  ScannerSettings,
  SubtitleOrigin,
  WatchSnapshot,
  WatchedVideo,
} from "../../types";

// How the scan gesture reads in the user's own configuration.
const MODIFIER_LABELS: Record<string, string> = {
  shift: "Hold Shift",
  ctrl: "Hold Ctrl",
  alt: "Hold Alt",
};

function scanHintFor(scanner: ScannerSettings): string {
  const prefix = MODIFIER_LABELS[scanner.modifier] ?? "Hold Shift";
  return scanner.modifier === "none"
    ? "Hover a word to look it up (needs Anki open)"
    : `${prefix} and hover a word to look it up (needs Anki open)`;
}

// Containers mpv plays that the app's own webview cannot — which is the whole reason the
// video is handed to mpv instead of being rendered here.
const VIDEO_EXTENSIONS = ["mkv", "mp4", "webm", "mov", "m4v", "avi", "ts", "flv"];
const SUBTITLE_EXTENSIONS = ["srt", "ass", "ssa", "vtt", "sub"];

// "" means "use the padding from Settings". Kept as a distinct choice rather than
// pre-filling the global value, so changing the setting later still applies here.
const PADDING_OPTIONS = [
  { value: "", label: "Default" },
  { value: "0", label: "None" },
  { value: "100", label: "100 ms" },
  { value: "250", label: "250 ms" },
  { value: "500", label: "500 ms" },
  { value: "750", label: "750 ms" },
  { value: "1000", label: "1000 ms" },
];

export function WatchPage({
  snapshot,
  isStarting,
  error,
  onStart,
  onStop,
  onMine,
  isMining,
  mineResult,
  mineHotkey,
  cues,
  subtitlesError,
  minedKeys,
  deckMinedKeys,
  miningKey,
  mineDisabledReason,
  onSeek,
  onMineLine,
  onMerge,
  onSplit,
  padBeforeMs,
  padAfterMs,
  onPadBeforeChange,
  onPadAfterChange,
  scanner,
  onToggleOverlay,
  onSetSubtitleDelay,
  hasJimakuKey,
  onSyncSubtitles,
  isSyncing,
  onGenerateSubtitles,
  videos,
  onAddVideo,
  onSubtitleChosen,
  onForgetVideo,
  onSearchJimaku,
  onRealign,
  missingVideoPaths,
  generatingPath,
  generateProgress,
  onCancelGenerate,
  openMenuPath,
  onOpenMenuChange,
  searchQuery,
  onSearchChange,
  syncResult,
}: {
  snapshot: WatchSnapshot;
  isStarting: boolean;
  error: string | null;
  onStart: (videoPath: string, subtitlePath: string | null) => void;
  onStop: () => void;
  onMine: () => void;
  isMining: boolean;
  // What the last mine said — success or the reason it failed. Shown rather than
  // swallowed, so a card that did not get made is never mistaken for one that did.
  mineResult: { ok: boolean; message: string } | null;
  // The registered global shortcut, or null if the OS refused every candidate.
  mineHotkey: string | null;
  cues: RecordingSegment[];
  subtitlesError: string | null;
  minedKeys: Set<string>;
  deckMinedKeys: Set<string>;
  miningKey: string | null;
  mineDisabledReason: string | null;
  onSeek: (positionMs: number) => void;
  onMineLine: (index: number) => void;
  onMerge: (index: number) => void;
  onSplit: (index: number) => void;
  padBeforeMs: string;
  padAfterMs: string;
  onPadBeforeChange: (value: string) => void;
  onPadAfterChange: (value: string) => void;
  scanner: ScannerSettings;
  onToggleOverlay: (enabled: boolean) => void;
  onSetSubtitleDelay: (delayMs: number) => void;
  hasJimakuKey: boolean;
  /// Undefined when there is no sidecar file to realign — an embedded track has no file of
  /// its own, and alass needs one to rewrite.
  onSyncSubtitles: (() => void) | undefined;
  isSyncing: boolean;
  // Transcribe the chosen video's own audio into a subtitle file beside it, for material
  // nothing has subtitles for. The result is adopted as the chosen sidecar, so Sync below
  // can then realign it exactly like a downloaded file.
  onGenerateSubtitles: (videoPath: string) => void;
  // The remembered videos, newest first. This page owns none of it — the list, the mapping
  // and the selection all live in App so they survive leaving the page, which is the entire
  // point of the feature.
  videos: WatchedVideo[];
  onAddVideo: (videoPath: string) => void;
  onSubtitleChosen: (
    videoPath: string,
    subtitlePath: string,
    origin: SubtitleOrigin,
  ) => void;
  onForgetVideo: (videoPath: string) => void;
  onSearchJimaku: (videoPath: string) => void;
  onRealign: (videoPath: string) => void;
  // Videos whose file could not be found. Listed and dimmed rather than hidden.
  missingVideoPaths: ReadonlySet<string>;
  // The video being transcribed, so its own row carries the bar.
  generatingPath: string | null;
  // 0-100 while subtitles are being written, null before the first tick arrives.
  generateProgress: number | null;
  onCancelGenerate: () => void;
  // Lifted so only one row's menu is open at a time.
  openMenuPath: string | null;
  onOpenMenuChange: (videoPath: string | null) => void;
  searchQuery: string;
  onSearchChange: (query: string) => void;
  syncResult: { ok: boolean; message: string } | null;
}) {
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  // The line on screen changes as the video plays. Keying the owner to the cue's start
  // means a highlight never survives onto a different line.
  const currentLineKey = `current:${snapshot.subtitleStartMs ?? 0}`;

  const pickVideo = async () => {
    const picked = await open({
      multiple: false,
      filters: [{ name: "Video", extensions: VIDEO_EXTENSIONS }],
    });
    if (typeof picked === "string") {
      onAddVideo(picked);
    }
  };

  const pickSubtitle = async (videoPath: string) => {
    const picked = await open({
      multiple: false,
      filters: [{ name: "Subtitles", extensions: SUBTITLE_EXTENSIONS }],
    });
    if (typeof picked === "string") {
      onSubtitleChosen(videoPath, picked, "picked");
    }
  };

  if (!snapshot.connected) {
    return (
      <div className="recorder-view">
        <article className="panel recent-panel">
          <header className="panel-header">
            <div>
              <p className="panel-kicker">Watch &amp; Mine</p>
              <h2>Video library</h2>
            </div>
            <div className="panel-actions">
              {videos.length > 0 ? (
                <input
                  type="search"
                  className="library-search"
                  value={searchQuery}
                  onChange={(event) => onSearchChange(event.target.value)}
                  placeholder="Search by name"
                  aria-label="Search videos by name"
                />
              ) : null}
              <button type="button" onClick={() => void pickVideo()}>
                Add a video
              </button>
            </div>
          </header>

          <p className="microcopy">
            Videos play in mpv. Whatever subtitles you pair with one are remembered.
          </p>

          <VideoLibraryList
            videos={videos}
            onOpen={(video) => onStart(video.videoPath, video.subtitlePath)}
            onChooseSubtitle={(video) => void pickSubtitle(video.videoPath)}
            onSearchJimaku={(video) => onSearchJimaku(video.videoPath)}
            onGenerateSubtitles={(video) => onGenerateSubtitles(video.videoPath)}
            onRealign={(video) => onRealign(video.videoPath)}
            onForget={(video) => onForgetVideo(video.videoPath)}
            missingPaths={missingVideoPaths}
            hasJimakuKey={hasJimakuKey}
            generatingPath={generatingPath}
            generateProgress={generateProgress}
            onCancelGenerate={onCancelGenerate}
            openMenuPath={openMenuPath}
            onOpenMenuChange={onOpenMenuChange}
            isStarting={isStarting}
            searchQuery={searchQuery}
          />

          {error ? (
            <div className="update-card error">
              <strong>{error}</strong>
            </div>
          ) : null}
        </article>
      </div>
    );
  }

  return (
    <div className="recorder-view">
      <article className="panel recent-panel">
      <header className="panel-header">
        <div>
          <p className="panel-kicker">Watch &amp; Mine</p>
          <h2>{snapshot.title ?? "Playing"}</h2>
        </div>
        <div className="panel-actions">
          <button type="button" className="secondary" onClick={onStop}>
            Stop
          </button>
        </div>
      </header>

      <div className="watch-transport">
        <span className="watch-clock">
          {formatDuration(snapshot.positionMs ?? 0)}
          {snapshot.durationMs ? ` / ${formatDuration(snapshot.durationMs)}` : ""}
        </span>
        <span
          className={`status-chip status-chip-${snapshot.paused ? "warning" : "success"}`}
        >
          {snapshot.paused ? "Paused" : "Playing"}
        </span>
      </div>

      {/* The line mpv currently has on screen. This is what the Mine button acts on, so
          showing it is also how you confirm the app and the player agree. */}
      <div className="watch-line">
        {snapshot.subtitleText ? (
          <>
            <p className="watch-line-text">
              <ScannableText ownerKey={currentLineKey}>
                {snapshot.subtitleText}
              </ScannableText>
            </p>
            <p className="microcopy">
              {formatDuration(snapshot.subtitleStartMs ?? 0)} &ndash;{" "}
              {formatDuration(snapshot.subtitleEndMs ?? 0)}
              {snapshot.subtitleDelayMs !== 0
                ? ` · offset ${snapshot.subtitleDelayMs > 0 ? "+" : ""}${snapshot.subtitleDelayMs}ms`
                : ""}
            </p>
          </>
        ) : (
          <p className="microcopy">No subtitle on screen right now.</p>
        )}
      </div>

      <div className="panel-actions watch-actions">
        <button
          type="button"
          onClick={onMine}
          disabled={isMining || !snapshot.subtitleText}
          title={
            snapshot.subtitleText
              ? "Make an Anki card from the line playing now"
              : "There is no line on screen to mine"
          }
        >
          {isMining ? "Mining…" : "Mine the current line"}
        </button>
        {/* The hotkey is the point — it fires while mpv has focus, so mining does not
            mean leaving the video. The button is for when you are already looking here. */}
        {mineHotkey ? (
          <span className="watch-hotkey-hint">
            or press <kbd>{mineHotkey}</kbd> without leaving mpv
          </span>
        ) : null}

        {/* Off by default: mpv's own .ass rendering — positioning, colours, karaoke — is
            what works today, and ours replaces it only when the user wants to scan over
            the video. mpv cannot report where a word is drawn, so there is no way to have
            both. */}
        <label className="toggle watch-overlay-toggle">
          <input
            type="checkbox"
            checked={scanner.overlayEnabled}
            onChange={(event) => onToggleOverlay(event.currentTarget.checked)}
          />
          <span>Scannable subtitles over the video</span>
        </label>

        {/* Padding is asymmetric because the two edges fail differently: a line's start is
            usually tight, while its end clips a trailing syllable. "Default" defers to the
            Settings value rather than copying it, so changing that later still applies. */}
        <label className="watch-padding">
          <span>Pad before</span>
          <ThemedSelect
            value={padBeforeMs}
            options={PADDING_OPTIONS}
            placeholder="Default"
            onChange={onPadBeforeChange}
          />
        </label>
        <label className="watch-padding">
          <span>Pad after</span>
          <ThemedSelect
            value={padAfterMs}
            options={PADDING_OPTIONS}
            placeholder="Default"
            onChange={onPadAfterChange}
          />
        </label>
      </div>

      {/* Subtitle timing. The offset is the cheap fix and comes first: it applies instantly,
          writes nothing, and undoes itself. Automatic alignment is for the case an offset
          cannot express — drift that varies across the episode. */}
      <div className="panel-actions watch-sync">
        <SubtitleOffsetField
          delayMs={snapshot.subtitleDelayMs}
          onCommit={onSetSubtitleDelay}
        />

        <button
          type="button"
          className="secondary"
          onClick={() => onSyncSubtitles?.()}
          disabled={!onSyncSubtitles || isSyncing}
          title={
            onSyncSubtitles
              ? "Realign the subtitle file against the video's audio, writing a copy beside it"
              : "Only a subtitle file can be realigned — an embedded track has no file to rewrite"
          }
        >
          {isSyncing ? "Aligning…" : "Sync automatically"}
        </button>
      </div>

      {syncResult ? (
        <div className={`update-card ${syncResult.ok ? "current" : "error"}`}>
          <strong>{syncResult.message}</strong>
        </div>
      ) : null}

      {mineResult ? (
        <div className={`update-card ${mineResult.ok ? "current" : "error"}`}>
          <strong>{mineResult.message}</strong>
        </div>
      ) : null}

      {subtitlesError ? (
        <div className="update-card error">
          <strong>{subtitlesError}</strong>
        </div>
      ) : null}

      <div className="transcript-viewer-body is-single">
        <SubtitleListPane
          cues={cues}
          positionMs={snapshot.positionMs}
          minedKeys={minedKeys}
          deckMinedKeys={deckMinedKeys}
          miningKey={miningKey}
          isMining={isMining}
          mineDisabledReason={mineDisabledReason}
          selectedKey={selectedKey}
          onSelect={setSelectedKey}
          onSeek={onSeek}
          onMine={onMineLine}
          onMerge={onMerge}
          onSplit={onSplit}
          scanHint={scanHintFor(scanner)}
        />
      </div>
      </article>
    </div>
  );
}
