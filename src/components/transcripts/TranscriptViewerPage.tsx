import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { useAudioPlayer } from "../../hooks/useAudioPlayer";
import { useRecordingTexts } from "../../hooks/useRecordingTexts";
import { formatBytes, formatDuration, formatTimestamp } from "../../lib/format";
import { transcriptLanguageLabel } from "../../lib/helpers";
import { ScannableText } from "../scanner/ScannableText";
import type {
  MinedLinesResult,
  RecentRecording,
  RecordingSegment,
  RecordingTextDocument,
  TranscriptionLiveSegment,
} from "../../types";
import { NowPlayingBar } from "../audio/NowPlayingBar";
import { TranscriptLanguageTabs } from "./TranscriptLanguageTabs";
import type { TranscriptLanguageTab } from "./TranscriptLanguageTabs";
import { TranscriptReadingPane, buildRows } from "./TranscriptReadingPane";
import { useSentenceRanking } from "../../hooks/useSentenceRanking";
import {
  countMatches,
  normalizeSegmentText,
  splitTranscriptSegments,
} from "./transcriptText";

type TranscriptViewMode = "sideBySide" | "transcript" | "translation";

const VIEW_MODES: { id: TranscriptViewMode; label: string }[] = [
  { id: "sideBySide", label: "Side by side" },
  { id: "transcript", label: "Transcript" },
  { id: "translation", label: "Translation" },
];

// Scripts without word spacing get a wider leading and a shorter measure.
const CJK_LANGUAGES = new Set(["ja", "zh", "yue", "zh-cn", "zh-tw"]);

// Sentence-ending punctuation used to pick a natural split point (CJK + Latin).
const SENTENCE_ENDINGS = new Set(["。", "！", "？", "．", ".", "!", "?", "…"]);

function segmentMineKey(segment: RecordingSegment): string {
  return `${segment.startMs}:${segment.endMs}:${segment.text}`;
}

function mergeSegmentAt(
  segments: RecordingSegment[],
  index: number,
  joiner: string,
): RecordingSegment[] {
  if (index < 0 || index >= segments.length - 1) {
    return segments;
  }
  const a = segments[index];
  const b = segments[index + 1];
  const merged: RecordingSegment = {
    text: `${a.text}${joiner}${b.text}`,
    startMs: a.startMs,
    endMs: b.endMs,
  };
  return [...segments.slice(0, index), merged, ...segments.slice(index + 2)];
}

function splitSegmentAt(
  segments: RecordingSegment[],
  index: number,
): RecordingSegment[] {
  const segment = segments[index];
  if (!segment) {
    return segments;
  }
  const text = segment.text;
  if (text.length < 2) {
    return segments;
  }

  const midpoint = Math.floor(text.length / 2);
  let cutIndex = midpoint;
  for (let position = midpoint; position < text.length; position += 1) {
    if (SENTENCE_ENDINGS.has(text[position])) {
      cutIndex = position + 1;
      break;
    }
  }

  if (cutIndex <= 0 || cutIndex >= text.length) {
    cutIndex = midpoint;
  }

  const firstText = text.slice(0, cutIndex).trim();
  const secondText = text.slice(cutIndex).trim();
  if (firstText.length === 0 || secondText.length === 0) {
    return segments;
  }

  const span = segment.endMs - segment.startMs;
  const splitMs = Math.round(segment.startMs + span * (cutIndex / text.length));
  const first: RecordingSegment = {
    text: firstText,
    startMs: segment.startMs,
    endMs: splitMs,
  };
  const second: RecordingSegment = {
    text: secondText,
    startMs: splitMs,
    endMs: segment.endMs,
  };
  return [
    ...segments.slice(0, index),
    first,
    second,
    ...segments.slice(index + 1),
  ];
}

const MISALIGNED_TRANSLATION = Symbol("misaligned-translation");

function pairedTranslationFor(
  index: number,
  segment: RecordingSegment,
  transcript: RecordingTextDocument | null,
  translation: RecordingTextDocument | null,
): string | null | typeof MISALIGNED_TRANSLATION {
  if (!translation || translation.missing) {
    return null;
  }
  const original = transcript?.segments[index];
  if (
    !original ||
    original.startMs !== segment.startMs ||
    original.endMs !== segment.endMs ||
    original.text !== segment.text
  ) {
    return null;
  }
  const lines = splitTranscriptSegments(translation.text);
  if (lines.length !== transcript?.segments.length) {
    return MISALIGNED_TRANSLATION;
  }
  const line = lines[index]?.trim();
  return line && line.length > 0 ? line : null;
}

function documentLanguageLabel(document: RecordingTextDocument): string {
  const requested =
    transcriptLanguageLabel(document.language) ??
    document.language.toUpperCase();
  if (document.language === "auto") {
    return transcriptLanguageLabel(document.detectedLanguage) ?? requested;
  }
  return requested;
}

function isCjkDocument(document: RecordingTextDocument | null): boolean {
  if (!document) {
    return false;
  }
  return (
    CJK_LANGUAGES.has(document.language) ||
    (document.detectedLanguage !== null &&
      CJK_LANGUAGES.has(document.detectedLanguage))
  );
}

function TranscriptSkeleton() {
  return (
    <div className="transcript-pane">
      <div className="transcript-pane-body">
        <div className="transcript-skeleton" aria-hidden="true">
          {[72, 96, 58, 88, 66, 92, 50].map((width, index) => (
            <span
              key={index}
              className="transcript-skeleton-line"
              style={{ width: `${width}%` }}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

function LiveTranscriptPane({
  segments,
}: {
  segments: TranscriptionLiveSegment[];
}) {
  const endRef = useRef<HTMLDivElement | null>(null);
  const bodyRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const body = bodyRef.current;
    if (!body) {
      return;
    }
    const distanceFromBottom =
      body.scrollHeight - body.scrollTop - body.clientHeight;
    if (distanceFromBottom < 120) {
      endRef.current?.scrollIntoView({ block: "nearest" });
    }
  }, [segments.length]);

  return (
    <div className="transcript-pane">
      <header className="transcript-pane-header">
        <div>
          <p className="panel-kicker">Transcribing</p>
          <h3>Live transcript</h3>
        </div>
        <span className="transcript-pane-note" aria-live="polite">
          {segments.length === 1
            ? "1 sentence so far"
            : `${segments.length} sentences so far`}
        </span>
      </header>
      <div className="transcript-pane-body" ref={bodyRef}>
        {segments.length === 0 ? (
          <p className="transcript-live-waiting">
            Waiting for the first sentence…
          </p>
        ) : null}
        <ol className="transcript-live-list" aria-live="off">
          {segments.map((segment, index) => (
            <li
              key={`${segment.startMs}-${segment.endMs}-${index}`}
              className="transcript-live-row"
            >
              <span className="transcript-live-time">
                {formatDuration(segment.startMs)}
              </span>
              <p className="transcript-live-text">
                <ScannableText ownerKey={`live:${segment.startMs}`}>
                  {segment.text}
                </ScannableText>
              </p>
            </li>
          ))}
        </ol>
        <div ref={endRef} />
      </div>
    </div>
  );
}

export function TranscriptViewerPage({
  recording,
  onBack,
  onReTranscribe,
  isReTranscribing,
  reTranscribeProgress,
  onReTranslate,
  isReTranslating,
  onMineSegment,
  isMining,
  expressionFieldMapped,
  ankiReachable,
  minedSentences,
  liveSegments,
  onCancelTranscription,
  lastTranscriptionOutcome,
  transcriptionLanguage,
  clipPaddingMs,
  allowDuplicateMinedWords,
  mineWordsWithoutContext,
  externallyMinedKeys,
  onLinesMined,
  knownWordsBuiltAtMs,
}: {
  recording: RecentRecording;
  onBack: () => void;
  onReTranscribe: ((force: boolean) => void) | undefined;
  isReTranscribing: boolean;
  reTranscribeProgress: number | null;
  onReTranslate: ((force: boolean) => void) | undefined;
  isReTranslating: boolean;
  onMineSegment: (
    text: string,
    startMs: number,
    endMs: number,
    translation: string | null,
  ) => Promise<boolean>;
  isMining: boolean;
  expressionFieldMapped: boolean;
  ankiReachable: boolean;
  minedSentences: Set<string>;
  liveSegments: TranscriptionLiveSegment[];
  onCancelTranscription: (() => void) | undefined;
  transcriptionLanguage: string;
  clipPaddingMs: number;
  allowDuplicateMinedWords: boolean;
  mineWordsWithoutContext: boolean;
  externallyMinedKeys: ReadonlySet<string>;
  onLinesMined: (keys: ReadonlySet<string>) => void;
  knownWordsBuiltAtMs: number | null;
  lastTranscriptionOutcome: { status: string; message?: string } | null;
}) {
  const changeSignature = `${recording.transcripts
    .map(
      (transcript) => `${transcript.language}:${transcript.segmentsPath ?? ""}`,
    )
    .join("|")}:${recording.translationPath ?? ""}`;
  const { data, status, error, reload } = useRecordingTexts({
    filePath: recording.filePath,
    changeSignature,
  });

  const isRewritingText = isReTranscribing || isReTranslating;
  const wasRewritingTextRef = useRef(false);
  useEffect(() => {
    if (wasRewritingTextRef.current && !isRewritingText) {
      reload();
    }
    wasRewritingTextRef.current = isRewritingText;
  }, [isRewritingText, reload]);

  const player = useAudioPlayer();
  const isActiveTrack = player.filePath === recording.filePath;
  const handleTogglePlayback = () => {
    if (isActiveTrack) {
      player.toggle();
    } else {
      player.playRecording(recording);
    }
  };
  const handleSeekPlayback = (ms: number) => {
    if (isActiveTrack) {
      player.seekMs(ms);
    } else {
      player.playRecording(recording);
    }
  };
  const handlePlaySegment = recording.audioDeleted
    ? undefined
    : (startMs: number, endMs: number) =>
        player.playSegment(
          recording,
          startMs,
          endMs,
          clipPaddingMs,
          (message) => toast.error(message),
        );
  const activeSegment = isActiveTrack ? player.activeSegment : null;

  const transcripts = data?.transcripts ?? [];
  const translations = data?.translations ?? [];

  const [activeLanguage, setActiveLanguage] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<TranscriptViewMode>("sideBySide");
  const [query, setQuery] = useState("");
  const [selectedSegment, setSelectedSegment] = useState<string | null>(null);
  const [activeSegmentIndex, setActiveSegmentIndex] = useState<number | null>(
    null,
  );

  const activeTranscript = useMemo<RecordingTextDocument | null>(() => {
    if (transcripts.length === 0) {
      return null;
    }
    for (const preferred of [activeLanguage, transcriptionLanguage]) {
      if (!preferred) {
        continue;
      }
      const match = transcripts.find((doc) => doc.language === preferred);
      if (match) {
        return match;
      }
    }
    return transcripts[0];
  }, [transcripts, activeLanguage, transcriptionLanguage]);

  const activeTranslation = translations[0] ?? null;

  const [editedSegments, setEditedSegments] = useState<RecordingSegment[]>([]);
  const [withinReachOnly, setWithinReachOnly] = useState(false);
  const allowDuplicateWords = allowDuplicateMinedWords;
  const [mineFailures, setMineFailures] = useState<Map<string, string>>(
    new Map(),
  );
  const [isBatchMining, setIsBatchMining] = useState(false);
  const transcriptLines = useMemo(
    () =>
      activeTranscript
        ? buildRows(activeTranscript, editedSegments).map((row) => row.text)
        : [],
    [activeTranscript, editedSegments],
  );
  const ranking = useSentenceRanking(transcriptLines, knownWordsBuiltAtMs);
  const [minedKeys, setMinedKeys] = useState<Set<string>>(new Set());
  const minedInThisSession = useMemo(
    () => new Set([...minedKeys, ...externallyMinedKeys]),
    [minedKeys, externallyMinedKeys],
  );

  useEffect(() => {
    onLinesMined(minedKeys);
  }, [minedKeys, onLinesMined]);
  const [miningKey, setMiningKey] = useState<string | null>(null);

  const minedKeysFromAnki = useMemo(
    () =>
      new Set(
        editedSegments
          .filter((segment) =>
            minedSentences.has(normalizeSegmentText(segment.text)),
          )
          .map(segmentMineKey),
      ),
    [editedSegments, minedSentences],
  );

  useEffect(() => {
    setEditedSegments(activeTranscript?.segments ?? []);
    setMinedKeys(new Set());
    setMineFailures(new Map());
    setMiningKey(null);
    setSelectedSegment(null);
    setActiveSegmentIndex(null);
  }, [activeTranscript]);

  const handleMergeSegment = (index: number) => {
    const joiner = isCjkDocument(activeTranscript) ? "" : " ";
    setEditedSegments((segments) => mergeSegmentAt(segments, index, joiner));
  };

  const handleSplitSegment = (index: number) => {
    setEditedSegments((segments) => splitSegmentAt(segments, index));
  };

  const handleMineSegment = (index: number) => {
    const segment = editedSegments[index];
    if (!segment || miningKey !== null) {
      return;
    }
    const key = segmentMineKey(segment);
    if (minedInThisSession.has(key)) {
      return;
    }
    setMiningKey(key);
    const paired = pairedTranslationFor(
      index,
      segment,
      activeTranscript,
      activeTranslation,
    );
    if (paired === MISALIGNED_TRANSLATION) {
      toast.warning(
        "This line is mined without a translation — the translation has a different number of lines, so no single line matches it.",
      );
    }
    const translation = paired === MISALIGNED_TRANSLATION ? null : paired;
    void onMineSegment(
      segment.text,
      segment.startMs,
      segment.endMs,
      translation,
    )
      .then((mined) => {
        if (mined) {
          setMinedKeys((previous) => {
            const next = new Set(previous);
            next.add(key);
            return next;
          });
        }
      })
      .catch((error: unknown) => {
        toast.error(
          typeof error === "string"
            ? error
            : "This sentence could not be mined.",
        );
      })
      .finally(() => {
        setMiningKey((current) => (current === key ? null : current));
      });
  };

  const minableWithinReach = useMemo(() => {
    if (!ranking || ranking.status !== "ready") {
      return [];
    }
    const candidates = editedSegments
      .map((segment, index) => ({ segment, index }))
      .filter(({ segment, index }) => {
        const key = segmentMineKey(segment);
        const line = ranking.lines[index];
        const learnable =
          (line?.withinReach ?? false) &&
          (mineWordsWithoutContext || (line?.hasContext ?? false));
        return (
          learnable &&
          !minedInThisSession.has(key) &&
          !minedKeysFromAnki.has(key)
        );
      });
    if (allowDuplicateWords) {
      return candidates;
    }

    const bestForWord = new Map<
      string,
      { segment: RecordingSegment; index: number }
    >();
    for (const candidate of candidates) {
      const line = ranking.lines[candidate.index];
      const word = line?.unknownWords[0];
      if (word === undefined) {
        continue;
      }
      const held = bestForWord.get(word);
      if (
        !held ||
        (ranking.lines[candidate.index]?.contentWordCount ?? 0) >
          (ranking.lines[held.index]?.contentWordCount ?? 0)
      ) {
        bestForWord.set(word, candidate);
      }
    }

    return [...bestForWord.values()].sort((a, b) => a.index - b.index);
  }, [
    ranking,
    editedSegments,
    minedInThisSession,
    minedKeysFromAnki,
    allowDuplicateWords,
    mineWordsWithoutContext,
  ]);

  const skippedWithinReach = useMemo(() => {
    if (!ranking || ranking.status !== "ready") {
      return 0;
    }
    // Counted over every line the filter SHOWS, which is every line one word away.
    const shown = ranking.lines.filter((line) => line.withinReach).length;
    return Math.max(0, shown - minableWithinReach.length);
  }, [ranking, minableWithinReach]);

  const handleMineWithinReach = async () => {
    if (minableWithinReach.length === 0 || isBatchMining) {
      return;
    }
    setIsBatchMining(true);
    setMineFailures(new Map());
    try {
      const result = await invoke<MinedLinesResult>("mine_segments_to_anki", {
        filePath: recording.filePath,
        lines: minableWithinReach.map(({ segment, index }) => {
          const paired = pairedTranslationFor(
            index,
            segment,
            activeTranscript,
            activeTranslation,
          );
          return {
            text: segment.text,
            startMs: segment.startMs,
            endMs: segment.endMs,
            translation: paired === MISALIGNED_TRANSLATION ? null : paired,
          };
        }),
      });

      const failures = new Map<string, string>();
      const mined = new Set(minedKeys);
      for (const line of result.lines) {
        const key = `${line.startMs}:${line.endMs}:${line.text}`;
        if (line.status === "added") {
          mined.add(key);
        } else if (line.status === "failed") {
          failures.set(key, line.message);
        }
      }
      setMinedKeys(mined);
      setMineFailures(failures);

      if (failures.size > 0) {
        toast.warning(
          `${result.message} The lines that failed are marked in the transcript.`,
        );
      } else {
        toast.success(result.message);
      }
    } catch (error: unknown) {
      toast.error(
        typeof error === "string"
          ? error
          : "These sentences could not be mined.",
      );
    } finally {
      setIsBatchMining(false);
    }
  };

  // Mining writes an Anki card with the sentence audio, so it needs local audio
  // present. When it isn't usable, an explanatory tooltip replaces the action.
  const mineDisabledReason = !expressionFieldMapped
    ? "Map an Anki note first"
    : !ankiReachable
      ? "Anki not reachable"
      : null;

  // Keyboard-driven mining.
  const keyboardStateRef = useRef({
    enabled: false,
    segments: editedSegments,
    selectedSegment,
    audioDeleted: recording.audioDeleted,
    mineBlocked: true,
    play: handlePlaySegment,
    mine: handleMineSegment,
  });

  useEffect(() => {
    keyboardStateRef.current = {
      enabled: viewMode !== "translation" && editedSegments.length > 0,
      segments: editedSegments,
      selectedSegment,
      audioDeleted: recording.audioDeleted,
      mineBlocked: mineDisabledReason !== null || isMining,
      play: handlePlaySegment,
      mine: handleMineSegment,
    };
  });

  useEffect(() => {
    const focusRow = (index: number) => {
      setSelectedSegment(`transcript-${index}`);
      setActiveSegmentIndex(index);
      document
        .querySelector(`[data-segment="transcript-${index}"]`)
        ?.scrollIntoView({ block: "nearest" });
    };
    const handler = (event: KeyboardEvent) => {
      const state = keyboardStateRef.current;
      if (!state.enabled || event.ctrlKey || event.metaKey || event.altKey) {
        return;
      }
      const focused = document.activeElement as HTMLElement | null;
      const tag = focused?.tagName;
      if (
        tag === "INPUT" ||
        tag === "TEXTAREA" ||
        tag === "SELECT" ||
        tag === "BUTTON" ||
        tag === "A" ||
        focused?.isContentEditable ||
        focused?.closest("[role='listbox'], [role='menu'], [role='dialog']")
      ) {
        return;
      }
      const count = state.segments.length;
      const raw =
        state.selectedSegment && state.selectedSegment.startsWith("transcript-")
          ? Number(state.selectedSegment.slice("transcript-".length))
          : Number.NaN;
      const current =
        Number.isInteger(raw) && raw >= 0 && raw < count ? raw : null;

      switch (event.key) {
        case "j":
        case "J":
        case "ArrowDown":
          event.preventDefault();
          focusRow(current === null ? 0 : Math.min(current + 1, count - 1));
          break;
        case "k":
        case "K":
        case "ArrowUp":
          event.preventDefault();
          focusRow(current === null ? count - 1 : Math.max(current - 1, 0));
          break;
        case " ": {
          if (current === null) {
            return;
          }
          event.preventDefault();
          const segment = state.segments[current];
          if (segment && state.play) {
            state.play(segment.startMs, segment.endMs);
          }
          break;
        }
        case "Enter":
          if (current === null) {
            return;
          }
          event.preventDefault();
          if (!state.audioDeleted && !state.mineBlocked) {
            state.mine(current);
          }
          break;
        default:
          break;
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  const languageTabs = useMemo<TranscriptLanguageTab[]>(
    () =>
      transcripts.map((doc) => ({
        code: doc.language,
        label: documentLanguageLabel(doc),
      })),
    [transcripts],
  );

  // Every match on screen, in reading order, as (pane, row, occurrence).
  const matches = useMemo(() => {
    const trimmed = query.trim();
    if (!trimmed) {
      return [];
    }
    const panes: { paneKey: "transcript" | "translation"; rows: string[] }[] =
      [];
    if (
      viewMode !== "translation" &&
      activeTranscript &&
      !activeTranscript.missing
    ) {
      panes.push({
        paneKey: "transcript",
        rows:
          withinReachOnly && ranking
            ? transcriptLines.map((text, index) =>
                ranking.lines[index]?.withinReach ? text : "",
              )
            : transcriptLines,
      });
    }
    if (
      viewMode !== "transcript" &&
      activeTranslation &&
      !activeTranslation.missing
    ) {
      panes.push({
        paneKey: "translation",
        rows: buildRows(activeTranslation, undefined).map((row) => row.text),
      });
    }

    const found: {
      paneKey: "transcript" | "translation";
      index: number;
      occurrence: number;
    }[] = [];
    for (const pane of panes) {
      pane.rows.forEach((text, index) => {
        for (
          let occurrence = 0;
          occurrence < countMatches(text, trimmed);
          occurrence += 1
        ) {
          found.push({ paneKey: pane.paneKey, index, occurrence });
        }
      });
    }
    return found;
  }, [
    viewMode,
    activeTranscript,
    activeTranslation,
    transcriptLines,
    query,
    withinReachOnly,
    ranking,
  ]);

  const matchCount = matches.length;
  const [activeMatchIndex, setActiveMatchIndex] = useState<number | null>(null);

  useEffect(() => {
    setActiveMatchIndex(null);
  }, [query, viewMode]);

  const stepMatch = (direction: 1 | -1) => {
    if (matches.length === 0) {
      return;
    }
    const next =
      activeMatchIndex === null
        ? direction === 1
          ? 0
          : matches.length - 1
        : (activeMatchIndex + direction + matches.length) % matches.length;
    setActiveMatchIndex(next);

    const match = matches[next];
    const row = document.querySelector(
      `[data-segment="${match.paneKey}-${match.index}"]`,
    );
    row?.scrollIntoView({ block: "center", behavior: "smooth" });
    if (match.paneKey === "transcript") {
      setActiveSegmentIndex(match.index);
    }
  };

  const activeMatch =
    activeMatchIndex === null ? null : matches[activeMatchIndex];

  const metaText = [
    formatDuration(recording.durationMs),
    formatBytes(recording.bytesWritten),
    formatTimestamp(recording.createdAtMs),
  ].join(" · ");

  const transcriptNote =
    activeTranscript &&
    activeTranscript.language === "auto" &&
    activeTranscript.detectedLanguage
      ? "Auto-detected"
      : null;

  const trimmedQuery = query.trim();

  const canEnablePerSentence =
    onReTranscribe !== undefined &&
    !recording.audioDeleted &&
    viewMode !== "translation" &&
    activeTranscript !== null &&
    !activeTranscript.missing &&
    activeTranscript.text.trim().length > 0 &&
    activeTranscript.segments.length === 0;

  const canReTranslate =
    onReTranslate !== undefined && recording.translationPath !== null;

  const canTranslate =
    onReTranslate !== undefined && recording.translationPath === null;

  const showKeyboardHint =
    viewMode !== "translation" &&
    !recording.audioDeleted &&
    editedSegments.length > 0;

  const canReTranscribe =
    onReTranscribe !== undefined &&
    !recording.audioDeleted &&
    !canEnablePerSentence;

  return (
    <div className="transcript-viewer">
      <header className="transcript-viewer-header">
        <div className="transcript-viewer-heading">
          <button
            type="button"
            className="ghost transcript-back"
            onClick={onBack}
          >
            {"←"} Back to recordings
          </button>
          <div className="transcript-viewer-title">
            <p className="panel-kicker">Transcript</p>
            <h2 title={recording.fileName}>{recording.fileName}</h2>
            <p className="transcript-viewer-meta">{metaText}</p>
          </div>
        </div>

        <div className="transcript-viewer-controls">
          {languageTabs.length >= 2 ? (
            <TranscriptLanguageTabs
              value={activeTranscript?.language ?? ""}
              tabs={languageTabs}
              onChange={setActiveLanguage}
            />
          ) : null}

          <div
            className="transcript-view-toggle"
            role="group"
            aria-label="Reading layout"
          >
            {VIEW_MODES.map((mode) => (
              <button
                key={mode.id}
                type="button"
                className={`transcript-view-toggle-button ${
                  viewMode === mode.id ? "is-active" : ""
                }`}
                aria-pressed={viewMode === mode.id}
                onClick={() => setViewMode(mode.id)}
              >
                {mode.label}
              </button>
            ))}
          </div>

          {ranking?.status === "ready" ? (
            <button
              type="button"
              className={`transcript-mode ${withinReachOnly ? "is-active" : ""}`}
              aria-pressed={withinReachOnly}
              onClick={() => setWithinReachOnly((current) => !current)}
              title={ranking.message}
            >
              One word away
            </button>
          ) : null}
          {withinReachOnly && !recording.audioDeleted ? (
            <button
              type="button"
              className="transcript-mode"
              onClick={() => void handleMineWithinReach()}
              disabled={
                isBatchMining ||
                mineDisabledReason !== null ||
                minableWithinReach.length === 0
              }
              title={
                mineDisabledReason ??
                (minableWithinReach.length === 0
                  ? "Every line here is already a card"
                  : skippedWithinReach > 0
                    ? `Make a card of every line shown. ${skippedWithinReach} skipped: already in your deck (marked "In deck"), teaching a word another line here already covers${
                        mineWordsWithoutContext
                          ? ""
                          : ', or standing alone — turn on "Mine words that appear on their own" in Anki Mapping to include those'
                      }.`
                    : "Make a card of every line shown, one word at a time")
              }
            >
              {isBatchMining
                ? "Mining…"
                : skippedWithinReach > 0
                  ? `Mine all ${minableWithinReach.length} · ${skippedWithinReach} skipped`
                  : `Mine all ${minableWithinReach.length}`}
            </button>
          ) : null}

          <div className="transcript-find">
            <input
              type="search"
              className="transcript-find-input"
              placeholder="Find in transcript"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key !== "Enter") {
                  return;
                }
                event.preventDefault();
                stepMatch(event.shiftKey ? -1 : 1);
              }}
              aria-label="Find in transcript"
            />
            {trimmedQuery ? (
              <>
                <span className="transcript-find-count">
                  {matchCount === 0
                    ? "No matches"
                    : activeMatchIndex === null
                      ? `${matchCount} match${matchCount === 1 ? "" : "es"}`
                      : `${activeMatchIndex + 1} of ${matchCount}`}
                </span>
                <button
                  type="button"
                  className="transcript-find-step"
                  onClick={() => stepMatch(-1)}
                  disabled={matchCount === 0}
                  title="Previous match (Shift+Enter)"
                  aria-label="Previous match"
                >
                  <span aria-hidden="true">{"↑"}</span>
                </button>
                <button
                  type="button"
                  className="transcript-find-step"
                  onClick={() => stepMatch(1)}
                  disabled={matchCount === 0}
                  title="Next match (Enter)"
                  aria-label="Next match"
                >
                  <span aria-hidden="true">{"↓"}</span>
                </button>
              </>
            ) : null}
          </div>
        </div>
      </header>

      {recording.audioDeleted ? (
        <p className="now-playing-unavailable">
          Local audio was deleted — playback is unavailable for this recording.
        </p>
      ) : (
        <NowPlayingBar
          variant="compact"
          fileName={recording.fileName}
          isPlaying={isActiveTrack && player.isPlaying}
          currentTimeMs={isActiveTrack ? player.currentTimeMs : 0}
          durationMs={
            isActiveTrack && player.durationMs > 0
              ? player.durationMs
              : recording.durationMs
          }
          onToggle={handleTogglePlayback}
          onSeek={handleSeekPlayback}
          playbackRate={player.playbackRate}
          onSetPlaybackRate={player.setPlaybackRate}
          isRepeating={player.isRepeating}
          onToggleRepeat={player.toggleRepeat}
        />
      )}

      {!isReTranscribing && lastTranscriptionOutcome ? (
        <p
          className={`transcript-run-outcome${
            lastTranscriptionOutcome.status === "failed" ? " is-error" : ""
          }`}
          role={
            lastTranscriptionOutcome.status === "failed" ? "alert" : undefined
          }
        >
          {lastTranscriptionOutcome.status === "cancelled"
            ? "Transcription cancelled — no transcript was written."
            : (lastTranscriptionOutcome.message ??
              "Transcription failed — no transcript was written.")}
        </p>
      ) : null}

      {canEnablePerSentence ||
      canReTranscribe ||
      canReTranslate ||
      canTranslate ||
      isReTranscribing ? (
        <div
          className={`transcript-enable-timing${
            isReTranscribing ? " is-transcribing" : ""
          }`}
        >
          {isReTranscribing ? (
            <>
              <span className="transcript-enable-timing-text">
                {reTranscribeProgress !== null
                  ? `Transcribing… ${reTranscribeProgress}%`
                  : "Queued to transcribe…"}
              </span>
              <div
                className="transcript-enable-timing-progress"
                role="progressbar"
                aria-label="Transcription progress"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={reTranscribeProgress ?? undefined}
              >
                <div className="progress-track" aria-hidden="true">
                  <div
                    className="progress-fill"
                    style={{ width: `${reTranscribeProgress ?? 0}%` }}
                  />
                </div>
              </div>
              {onCancelTranscription ? (
                <button
                  type="button"
                  className="ghost transcript-enable-timing-cancel"
                  onClick={onCancelTranscription}
                >
                  Cancel
                </button>
              ) : null}
            </>
          ) : (
            <>
              <span className="transcript-enable-timing-text">
                {canEnablePerSentence
                  ? "Enable per-sentence playback — re-transcribe with timestamps."
                  : canReTranscribe
                    ? "Re-transcribe this recording — e.g. after switching Audio type to Music."
                    : canTranslate
                      ? "Translate this recording with the browser extension."
                      : "Re-run the translation for this recording."}
              </span>
              <div className="transcript-enable-timing-buttons">
                {canEnablePerSentence || canReTranscribe ? (
                  <button
                    type="button"
                    className="transcript-enable-timing-action"
                    onClick={() => onReTranscribe?.(true)}
                    title="Re-run transcription with the current settings — e.g. after switching Audio type to Music"
                  >
                    Re-transcribe
                  </button>
                ) : null}
                {canTranslate ? (
                  <button
                    type="button"
                    className="transcript-enable-timing-action"
                    onClick={() => onReTranslate?.(false)}
                    disabled={isReTranslating}
                  >
                    {isReTranslating ? "Translating…" : "Translate"}
                  </button>
                ) : null}
                {canReTranslate ? (
                  <button
                    type="button"
                    className="transcript-enable-timing-action"
                    onClick={() => onReTranslate?.(true)}
                    disabled={isReTranslating}
                  >
                    {isReTranslating ? "Re-translating…" : "Re-translate"}
                  </button>
                ) : null}
              </div>
            </>
          )}
        </div>
      ) : null}

      {showKeyboardHint ? (
        <p className="transcript-kbd-hint">
          <kbd>J</kbd>
          <span aria-hidden="true"> / </span>
          <kbd>K</kbd> move
          <span className="transcript-kbd-sep" aria-hidden="true">
            ·
          </span>
          <kbd>Space</kbd> play
          <span className="transcript-kbd-sep" aria-hidden="true">
            ·
          </span>
          <kbd>Enter</kbd> mine
        </p>
      ) : null}

      {status === "error" ? (
        <div className="transcript-viewer-body is-single">
          <div className="transcript-error">
            <p className="panel-kicker">Could not load</p>
            <p>{error}</p>
            <button type="button" className="secondary" onClick={reload}>
              Try again
            </button>
          </div>
        </div>
      ) : isReTranscribing ? (
        <div className="transcript-viewer-body is-single">
          <LiveTranscriptPane segments={liveSegments} />
        </div>
      ) : status === "loading" || data === null ? (
        <div className="transcript-viewer-body is-single">
          <TranscriptSkeleton />
        </div>
      ) : (
        <div
          className={`transcript-viewer-body ${
            viewMode === "sideBySide" ? "is-split" : "is-single"
          }`}
        >
          {viewMode !== "translation" ? (
            <TranscriptReadingPane
              paneKey="transcript"
              kicker="Transcript"
              title={
                activeTranscript
                  ? documentLanguageLabel(activeTranscript)
                  : "Transcript"
              }
              note={transcriptNote}
              isCjk={isCjkDocument(activeTranscript)}
              document={activeTranscript}
              query={query}
              emptyLabel="No transcript text yet."
              noSpeechLabel="No speech was detected in this recording."
              missingLabel="The transcript file is missing from this machine."
              selectedSegment={selectedSegment}
              onSelectSegment={setSelectedSegment}
              activeSegmentIndex={activeSegmentIndex}
              onActivateSegment={setActiveSegmentIndex}
              activeSegment={activeSegment}
              onPlaySegment={handlePlaySegment}
              editable
              segmentsOverride={editedSegments}
              onMergeSegment={handleMergeSegment}
              onSplitSegment={handleSplitSegment}
              onMineSegment={
                recording.audioDeleted ? undefined : handleMineSegment
              }
              minedKeys={minedInThisSession}
              deckMinedKeys={minedKeysFromAnki}
              miningKey={miningKey}
              isMining={isMining}
              mineDisabledReason={mineDisabledReason}
              ranking={ranking}
              withinReachOnly={withinReachOnly}
              mineFailures={mineFailures}
              activeMatch={
                activeMatch?.paneKey === "transcript" ? activeMatch : null
              }
            />
          ) : null}

          {viewMode !== "transcript" ? (
            <TranscriptReadingPane
              paneKey="translation"
              kicker="Translation"
              title={
                activeTranslation
                  ? documentLanguageLabel(activeTranslation)
                  : "Not translated"
              }
              note={null}
              isCjk={isCjkDocument(activeTranslation)}
              document={activeTranslation}
              query={query}
              emptyLabel="No translation yet. Use Translate on the recording to create one."
              missingLabel="The translation file is missing from this machine."
              selectedSegment={selectedSegment}
              onSelectSegment={setSelectedSegment}
              activeSegmentIndex={activeSegmentIndex}
              onActivateSegment={setActiveSegmentIndex}
              activeSegment={null}
              onPlaySegment={undefined}
            />
          ) : null}
        </div>
      )}
    </div>
  );
}
