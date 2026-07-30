import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { formatDuration, fileNameFromPath } from "../../lib/format";
import { ThemedSelect } from "../ui/ThemedSelect";
import { SubtitleListPane } from "./SubtitleListPane";
import { ScannableText } from "../scanner/ScannableText";
import { SubtitleOffsetField } from "./SubtitleOffsetField";
import { JimakuSearchPanel } from "./JimakuSearchPanel";
import type { RecordingSegment, ScannerSettings, WatchSnapshot } from "../../types";

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
  onOpenScannerSettings,
  onSyncSubtitles,
  isSyncing,
  onGenerateSubtitles,
  isGeneratingSubtitles,
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
  onOpenScannerSettings: () => void;
  /// Undefined when there is no sidecar file to realign — an embedded track has no file of
  /// its own, and alass needs one to rewrite.
  onSyncSubtitles: (() => void) | undefined;
  isSyncing: boolean;
  // Transcribe the chosen video's own audio into a subtitle file beside it, for material
  // nothing has subtitles for. The result is adopted as the chosen sidecar, so Sync below
  // can then realign it exactly like a downloaded file.
  onGenerateSubtitles: (videoPath: string) => void;
  isGeneratingSubtitles: boolean;
  syncResult: { ok: boolean; message: string } | null;
}) {
  const [videoPath, setVideoPath] = useState<string | null>(null);
  const [subtitlePath, setSubtitlePath] = useState<string | null>(null);
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
      setVideoPath(picked);
    }
  };

  const pickSubtitle = async () => {
    const picked = await open({
      multiple: false,
      filters: [{ name: "Subtitles", extensions: SUBTITLE_EXTENSIONS }],
    });
    if (typeof picked === "string") {
      setSubtitlePath(picked);
    }
  };

  if (!snapshot.connected) {
    return (
      <>
        <header className="panel-header">
          <div>
            <p className="panel-kicker">Watch &amp; Mine</p>
            <h2>Open a video</h2>
          </div>
        </header>

        <div className="info-note">
          <p className="microcopy">
            The video plays in <strong>mpv</strong>, not in this window &mdash; mpv handles
            MKV, H.265 and everything else this app&rsquo;s built-in player cannot. Wonder
            of U reads the position and the subtitles from it, so you can mine the line you
            are hearing.
          </p>
        </div>

        <div className="settings-grid">
          <label className="field">
            <span>Video</span>
            <button type="button" className="secondary" onClick={() => void pickVideo()}>
              {videoPath ? fileNameFromPath(videoPath) : "Choose a video…"}
            </button>
          </label>

          <label className="field">
            <span>Subtitles (optional)</span>
            <button type="button" className="secondary" onClick={() => void pickSubtitle()}>
              {subtitlePath ? fileNameFromPath(subtitlePath) : "Choose a subtitle file…"}
            </button>
          </label>
        </div>

        <p className="microcopy">
          Leave subtitles empty if the file already has them built in &mdash; the track is
          read straight out of the container.
        </p>

        <JimakuSearchPanel
          hasApiKey={hasJimakuKey}
          videoPath={videoPath}
          onDownloaded={setSubtitlePath}
          onOpenSettings={onOpenScannerSettings}
        />

        <div className="panel-section">
          <h3>No subtitles anywhere?</h3>
          <p className="microcopy">
            Write them from the video's own speech. This runs the same transcription the
            library uses, so it takes a few minutes on a full episode and needs the Whisper
            engine set up. The file is saved next to the video and selected for you.
          </p>
          <div className="panel-actions">
            <button
              type="button"
              className="secondary"
              onClick={() => videoPath && onGenerateSubtitles(videoPath)}
              disabled={!videoPath || isGeneratingSubtitles}
            >
              {isGeneratingSubtitles
                ? "Writing subtitles…"
                : "Generate subtitles from the audio"}
            </button>
          </div>
        </div>

        <div className="panel-actions">
          <button
            type="button"
            onClick={() => videoPath && onStart(videoPath, subtitlePath)}
            disabled={!videoPath || isStarting}
          >
            {isStarting ? "Opening…" : "Open in mpv"}
          </button>
        </div>

        {error ? (
          <div className="update-card error">
            <strong>{error}</strong>
          </div>
        ) : null}
      </>
    );
  }

  return (
    <>
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

    </>
  );
}
