import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { useAudioPlayer } from "../../hooks/useAudioPlayer";
import { useRecordingTexts } from "../../hooks/useRecordingTexts";
import {
  formatBytes,
  formatDuration,
  formatTimestamp,
} from "../../lib/format";
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
const SENTENCE_ENDINGS = new Set([
  "。",
  "！",
  "？",
  "．",
  ".",
  "!",
  "?",
  "…",
]);

// A stable, content-derived key for a segment so an already-mined row keeps its
// "✓ Mined" marker across re-renders. Merging/splitting produces a new sentence
// (new text/timing), so its key differs and the marker naturally resets.
function segmentMineKey(segment: RecordingSegment): string {
  return `${segment.startMs}:${segment.endMs}:${segment.text}`;
}

// Merge row i with row i+1 into one sentence spanning both time ranges. The
// joiner is script-aware: CJK scripts run without inter-word spaces, so a space
// would leave an unnatural gap in the merged sentence (and in a mined card).
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

// Split row i at the first sentence-ending punctuation at or after the text
// midpoint, else at the character midpoint. Time is divided proportionally by
// the character cut index so each half keeps a plausible span.
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
      // Keep the punctuation with the first sentence.
      cutIndex = position + 1;
      break;
    }
  }
  // A punctuation mark sitting at the very end leaves nothing for the second
  // half; fall back to the character midpoint in that case.
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
  return [...segments.slice(0, index), first, second, ...segments.slice(index + 1)];
}

/// Returned when a translation exists but its lines do not correspond to the transcript's,
/// so no line can be attached. Distinct from `null`, which means there is no translation at
/// all — the first is worth telling the reader about, the second is not.
const MISALIGNED_TRANSLATION = Symbol("misaligned-translation");

// The translation that already exists for a mined sentence: the positionally
// paired line the viewer shows beside it. Returns null (mine the text alone,
// never generate a fresh translation) when there is no translation document, or
// when the row was merged/split — an edit shifts the row out of alignment with
// the translation's lines, so the pairing can no longer be trusted.
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
  // The pairing is positional, so it is only meaningful when the two sides have the same
  // number of lines. A whole-document translation re-segments freely — one Japanese line
  // can come back as three English ones — and then row i on one side is simply not the
  // counterpart of row i on the other. Attaching it anyway put a confidently wrong sentence
  // on the card, which is worse than attaching none: nothing on the card says it is wrong.
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

// The transcript as it is being decoded. Deliberately read-only — no mining, no
// merge/split, no per-sentence playback: none of it is anchored to a saved transcript
// yet, and offering an action that would be undone seconds later is worse than not
// offering it. The pane sticks to the newest line so the text scrolls itself.
function LiveTranscriptPane({
  segments,
}: {
  segments: TranscriptionLiveSegment[];
}) {
  const endRef = useRef<HTMLDivElement | null>(null);
  const bodyRef = useRef<HTMLDivElement | null>(null);
  // Stick to the newest line ONLY while the reader is already at the bottom. Scrolling
  // unconditionally would yank them back down every second or two, making it impossible
  // to read back over an earlier sentence while the run continues.
  useEffect(() => {
    const body = bodyRef.current;
    if (!body) {
      return;
    }
    const distanceFromBottom =
      body.scrollHeight - body.scrollTop - body.clientHeight;
    // Generous threshold: a row lands between the measurement and this effect, so an
    // exact-bottom test would already read as "scrolled up".
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
        {/* The count is the live region, not the list: announcing every appended row
            (timestamp included) would read hundreds of sentences aloud with no way to
            stop it. */}
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
  knownWordsBuiltAtMs,
}: {
  recording: RecentRecording;
  onBack: () => void;
  // Force a re-transcribe of this recording for the active language so an older
  // transcript can be backfilled with timestamps. Undefined disables the
  // affordance entirely.
  onReTranscribe: ((force: boolean) => void) | undefined;
  isReTranscribing: boolean;
  // Percent (0–100) while this recording is the active re-transcription, or null when it's
  // queued / not transcribing — drives the in-viewer progress bar.
  reTranscribeProgress: number | null;
  // Force a re-translate of this recording (overwrites the existing translation).
  // Undefined disables the affordance.
  onReTranslate: ((force: boolean) => void) | undefined;
  isReTranslating: boolean;
  // Mine a single sentence into its own Anki card. Resolves true when a card was
  // actually created, so the row can show a persistent "✓ Mined" marker. The
  // paired translation line (or null when the recording has none) rides along so
  // mining reuses the existing translation instead of generating a fresh one.
  onMineSegment: (
    text: string,
    startMs: number,
    endMs: number,
    translation: string | null,
  ) => Promise<boolean>;
  isMining: boolean;
  // Whether the Anki expression field is mapped and Anki is reachable. Together
  // they decide whether Mine is enabled and which tooltip explains a disabled one.
  expressionFieldMapped: boolean;
  ankiReachable: boolean;
  // Normalized sentences already mined into the Anki deck, from any past session.
  // Empty when Anki is closed or the note type is unmapped, which simply means no
  // row is marked — never an error.
  minedSentences: Set<string>;
  // Sentences streamed from the whisper pass currently transcribing THIS recording.
  // Empty when nothing is running, or when the running transcription belongs to
  // another file in the queue.
  liveSegments: TranscriptionLiveSegment[];
  // Stop the running transcription. Undefined when this recording is not the one
  // being transcribed, which is also when the progress block is not rendered.
  onCancelTranscription: (() => void) | undefined;
  /// The transcription language from Settings. A recording can hold several transcript
  /// variants, and this is what decides which one is "the" transcript — the same setting a
  /// push reads, so what is on screen is what a push sends. The viewer used to open
  /// whichever variant happened to be first in the list, which is how the two came to
  /// disagree without either being obviously wrong.
  transcriptionLanguage: string;
  // Milliseconds the miner pads a clip by on each side. Playback uses the same value so a
  // previewed sentence and the card made from it cannot drift apart.
  clipPaddingMs: number;
  // Whether "Mine all" may make more than one card for the same new word. Changes
  // the count on the button as well as what it mines, so it is read here rather
  // than only at mining time.
  allowDuplicateMinedWords: boolean;
  // When the known-word list was last read from Anki. Only a re-rank trigger: a
  // Refresh has to update the badges on a transcript already open, or the words
  // learned this morning would not show until the page was left and returned to.
  knownWordsBuiltAtMs: number | null;
  // Set when the most recent transcription of this recording ended badly, so the viewer
  // can say which of "you cancelled it", "it failed" and "there is no transcript" the
  // empty screen actually means. Null when the last run succeeded or none has run.
  lastTranscriptionOutcome: { status: string; message?: string } | null;
}) {
  // The segments sidecar path is folded in so backfilling timestamps on an
  // already-transcribed language (same count, same translation) still changes
  // the signature and triggers a re-read once the sidecar lands.
  const changeSignature = `${recording.transcripts
    .map((transcript) => `${transcript.language}:${transcript.segmentsPath ?? ""}`)
    .join("|")}:${recording.translationPath ?? ""}`;
  const { data, status, error, reload } = useRecordingTexts({
    filePath: recording.filePath,
    changeSignature,
  });

  // `changeSignature` is built from sidecar PATHS, and every writer here overwrites the
  // path it already used — so a re-run is invisible to it by construction. That was known
  // for re-transcription and handled; re-translation has exactly the same shape and was
  // not, which is why a successful re-translate left the previous translation on screen.
  // The first translation did update, because the path went from null to set.
  //
  // So this watches every writer at once rather than growing a ref per writer: any work
  // that can rewrite this recording's text forces the re-read as it finishes, and a
  // future writer joins by being named in this one expression.
  const isRewritingText = isReTranscribing || isReTranslating;
  const wasRewritingTextRef = useRef(false);
  useEffect(() => {
    if (wasRewritingTextRef.current && !isRewritingText) {
      reload();
    }
    wasRewritingTextRef.current = isRewritingText;
  }, [isRewritingText, reload]);

  // Whole-file playback for this recording, driven by the compact top bar.
  // Gated on audioDeleted below so a transcript-only entry never tries to load.
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
      // Nothing loaded yet — start the track so the scrub has audio to move.
      player.playRecording(recording);
    }
  };
  // Per-sentence playback rides the same player as the top bar. Disabled when
  // the local audio is gone, so timed rows still show their timestamp but no
  // play control rather than pretending playback works.
  const handlePlaySegment = recording.audioDeleted
    ? undefined
    : (startMs: number, endMs: number) =>
        // The miner's own padding, so what you hear here is what the card will hold.
        // A sentence that cannot be cut says so rather than falling back to the
        // inaccurate seek it replaced.
        player.playSegment(recording, startMs, endMs, clipPaddingMs, (message) =>
          toast.error(message),
        );
  const activeSegment = isActiveTrack ? player.activeSegment : null;

  const transcripts = data?.transcripts ?? [];
  const translations = data?.translations ?? [];

  const [activeLanguage, setActiveLanguage] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<TranscriptViewMode>("sideBySide");
  const [query, setQuery] = useState("");
  const [selectedSegment, setSelectedSegment] = useState<string | null>(null);
  // Links transcript row i to translation row i by POSITION only. Today's
  // translation is a whole-document translation, so row i on one side is not
  // guaranteed to be the semantic counterpart of row i on the other — the
  // pairing is purely positional. Exact per-line alignment arrives with
  // per-segment translation; there is no semantic matching here.
  const [activeSegmentIndex, setActiveSegmentIndex] = useState<number | null>(
    null,
  );

  const activeTranscript = useMemo<RecordingTextDocument | null>(() => {
    if (transcripts.length === 0) {
      return null;
    }
    // What the reader picked wins; otherwise the Settings language, which is what a push
    // reads. Only when the recording has no variant for it does this fall back to the first
    // one, so "there is nothing in your language" still shows something rather than nothing.
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

  // A local, in-session editable copy of the active transcript's timed segments.
  // Merge/split rewrite this copy only; nothing is persisted, and switching
  // language or reloading the transcript resets it from the source segments.
  const [editedSegments, setEditedSegments] = useState<RecordingSegment[]>([]);
  // Narrows the transcript to the lines a single word from being readable.
  const [withinReachOnly, setWithinReachOnly] = useState(false);
  const allowDuplicateWords = allowDuplicateMinedWords;
  // Why a batch mine could not make a card of a row, keyed like the mined markers.
  const [mineFailures, setMineFailures] = useState<Map<string, string>>(
    new Map(),
  );
  const [isBatchMining, setIsBatchMining] = useState(false);
  // The same rows the transcript pane will build, so entry N of the ranking
  // describes row N. Merging or splitting a sentence changes these and re-ranks,
  // which is the point of ranking the lines rather than the sidecar.
  const transcriptLines = useMemo(
    () =>
      activeTranscript
        ? buildRows(activeTranscript, editedSegments).map((row) => row.text)
        : [],
    [activeTranscript, editedSegments],
  );
  const ranking = useSentenceRanking(transcriptLines, knownWordsBuiltAtMs);
  // Rows already mined, tracked by content key so the marker survives re-renders
  // but not a merge/split (which makes a new sentence). Seeded below from the
  // cards actually in Anki, so it covers earlier sessions too, then extended as
  // the user mines.
  const [minedKeys, setMinedKeys] = useState<Set<string>>(new Set());
  // The single row with a mine request in flight, so only it shows "Mining…".
  const [miningKey, setMiningKey] = useState<string | null>(null);

  // Rows whose sentence is already a card in the Anki mining deck. Derived rather
  // than stored, because `minedSentences` arrives from Anki asynchronously and the
  // segments change under merge/split — recomputing keeps both in step without an
  // effect that would clobber the in-session edits.
  //
  // These rows are flagged but stay mineable, which is why they are kept apart from
  // the session `minedKeys` that do spend the action: matching is on sentence text
  // across the whole deck, so a short recurring line would otherwise become
  // permanently unmineable everywhere once mined from any one recording.
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
    // A new transcript reindexes every row, so drop the old focus/selection.
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
    // Mined during this session — the row shows "✓ Mined" and hides its Mine button
    // in the mouse UI, so the keyboard path must refuse the duplicate too. A row
    // matched only against the deck is deliberately NOT refused: it still offers
    // "Mine again", and Enter has to agree with the button.
    if (minedKeys.has(key)) {
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
      // Said before the card is made, not after: the reader is about to get a card without
      // the translation they can see on screen, and the reason is not guessable from the
      // card itself.
      toast.warning(
        "This line is mined without a translation — the translation has a different number of lines, so no single line matches it.",
      );
    }
    const translation = paired === MISALIGNED_TRANSLATION ? null : paired;
    void onMineSegment(segment.text, segment.startMs, segment.endMs, translation)
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
        // The handler catches its own Anki errors and answers `null`, so nothing reaches here
        // today. It is here because `.finally` clears the spinner either way: without this, a
        // throw introduced upstream would look exactly like a mine that quietly did nothing —
        // button returns to normal, no card, no message.
        toast.error(
          typeof error === "string" ? error : "This sentence could not be mined.",
        );
      })
      .finally(() => {
        setMiningKey((current) => (current === key ? null : current));
      });
  };

  // The rows the "Mine all" action would act on: within reach, and not already a
  // card. Already-mined rows are skipped rather than refused — mining one at a time
  // offers "Mine again" deliberately, but a bulk run is not reviewed card by card,
  // and quietly doubling forty notes is not a thing to make easy.
  const minableWithinReach = useMemo(() => {
    if (!ranking || ranking.status !== "ready") {
      return [];
    }
    const candidates = editedSegments
      .map((segment, index) => ({ segment, index }))
      .filter(({ segment, index }) => {
        const key = segmentMineKey(segment);
        return (
          (ranking.lines[index]?.withinReach ?? false) &&
          !minedKeys.has(key) &&
          !minedKeysFromAnki.has(key)
        );
      });
    if (allowDuplicateWords) {
      return candidates;
    }

    // One line per new word. A transcript teaches a word twice often enough to
    // matter — a song repeats its lines, and this one has 生まれ変わる in two — and
    // two cards for one word is review load without extra learning.
    //
    // The line with the MOST content words wins, earliest on a tie. Same reasoning
    // as "i+1 needs an i": of two sentences a word away, the one with more around
    // it is the one you can infer the word from.
    const bestForWord = new Map<string, { segment: RecordingSegment; index: number }>();
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
    // Back into recording order: the list is read alongside the audio, and mining
    // is reported against it.
    return [...bestForWord.values()].sort((a, b) => a.index - b.index);
  }, [
    ranking,
    editedSegments,
    minedKeys,
    minedKeysFromAnki,
    allowDuplicateWords,
  ]);

  // How many one-word-away lines the batch will NOT mine because the sentence is
  // already a card. Counted so the difference can be SAID: the transcript shows a
  // badge on every line within reach, the button offers fewer, and without this the
  // gap between the two numbers has no explanation anywhere on screen.
  const skippedWithinReach = useMemo(() => {
    if (!ranking || ranking.status !== "ready") {
      return 0;
    }
    const withinReach = ranking.lines.filter((line) => line.withinReach).length;
    return Math.max(0, withinReach - minableWithinReach.length);
  }, [ranking, minableWithinReach]);

  const handleMineWithinReach = async () => {
    if (minableWithinReach.length === 0 || isBatchMining) {
      return;
    }
    setIsBatchMining(true);
    // Cleared first: a marker left from the previous run beside a line this run
    // succeeded on would be a lie about the state of the deck.
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
        // The count in the toast, the reasons on the rows. A toast holding three
        // lines of Japanese and three error messages is a toast nobody reads.
        toast.warning(
          `${result.message} The lines that failed are marked in the transcript.`,
        );
      } else {
        toast.success(result.message);
      }
    } catch (error: unknown) {
      toast.error(
        typeof error === "string" ? error : "These sentences could not be mined.",
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

  // Keyboard-driven mining. The once-registered keydown listener reads live state
  // through this ref so it never re-subscribes on every selection change nor holds
  // a stale closure. j/k (or ↓/↑) move a focused sentence, Space replays it, Enter
  // mines it — all reusing the same handlers the row buttons call.
  const keyboardStateRef = useRef({
    enabled: false,
    segments: editedSegments,
    selectedSegment,
    audioDeleted: recording.audioDeleted,
    mineBlocked: true,
    play: handlePlaySegment,
    mine: handleMineSegment,
  });
  // Sync the ref after each render (not during it, which React discourages) so the
  // once-registered keydown listener always reads the latest state.
  useEffect(() => {
    keyboardStateRef.current = {
      // Only when the transcript's timed sentences are actually on screen.
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
      // Never hijack typing or a focused control (search box, buttons, the speed
      // dropdown, the language tabs, links).
      const focused = document.activeElement as HTMLElement | null;
      const tag = focused?.tagName;
      if (
        tag === "INPUT" ||
        tag === "TEXTAREA" ||
        tag === "SELECT" ||
        tag === "BUTTON" ||
        tag === "A" ||
        focused?.isContentEditable ||
        // An open popup (e.g. the speed dropdown's listbox) owns arrow/enter keys.
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
  //
  // Built from the RENDERED rows rather than from the document's plain text. The
  // count used to come from the text while the rows come from the segments
  // sidecar, which are not always the same lines — tolerable for a number nobody
  // navigates, but "3 of 27" has to point at a row that exists.
  const matches = useMemo(() => {
    const trimmed = query.trim();
    if (!trimmed) {
      return [];
    }
    const panes: { paneKey: "transcript" | "translation"; rows: string[] }[] = [];
    if (viewMode !== "translation" && activeTranscript && !activeTranscript.missing) {
      panes.push({
        paneKey: "transcript",
        // A row the "One word away" filter has hidden has no element to scroll to
        // and no mark to light up, so counting its matches would mean a find bar
        // reading "5 of 27" and, on some of those, doing nothing at all. Blanked
        // rather than dropped, so the indices still line up with the rendered rows.
        rows:
          withinReachOnly && ranking
            ? transcriptLines.map((text, index) =>
                ranking.lines[index]?.withinReach ? text : "",
              )
            : transcriptLines,
      });
    }
    if (viewMode !== "transcript" && activeTranslation && !activeTranslation.missing) {
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
  // Which match Enter / the arrows are sitting on. Null means "found them, not
  // stepping through them yet", which is what a fresh query should look like.
  const [activeMatchIndex, setActiveMatchIndex] = useState<number | null>(null);

  useEffect(() => {
    setActiveMatchIndex(null);
  }, [query, viewMode]);

  const stepMatch = (direction: 1 | -1) => {
    if (matches.length === 0) {
      return;
    }
    // Wraps, like every find bar: reaching the end and being told "no more" when
    // there are matches above you is a dead end, not an answer.
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
    // `center` rather than `nearest`: a match one row below the fold would
    // otherwise scroll just barely into view at the very bottom, which reads as
    // nothing having happened.
    row?.scrollIntoView({ block: "center", behavior: "smooth" });
    if (match.paneKey === "transcript") {
      setActiveSegmentIndex(match.index);
    }
  };

  const activeMatch = activeMatchIndex === null ? null : matches[activeMatchIndex];

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

  // An older transcript with text but no timed segments can be backfilled by a
  // forced re-transcribe. Gated on local audio existing (nothing to re-run
  // without it) and on the transcript view being visible.
  const canEnablePerSentence =
    onReTranscribe !== undefined &&
    !recording.audioDeleted &&
    viewMode !== "translation" &&
    activeTranscript !== null &&
    !activeTranscript.missing &&
    activeTranscript.text.trim().length > 0 &&
    activeTranscript.segments.length === 0;

  // Re-run the (whole-document) translation, overwriting the existing sidecar.
  // Sits beside the re-transcribe action in the same bar.
  const canReTranslate =
    onReTranslate !== undefined && recording.translationPath !== null;

  // First-time translation for a recording that has never been translated. The
  // two are mutually exclusive on `translationPath`: an untranslated recording
  // shows "Translate" (force: false), a translated one shows "Re-translate".
  const canTranslate =
    onReTranslate !== undefined && recording.translationPath === null;

  // The keyboard shortcuts act on timed, mineable sentences with local audio, so
  // the hint only shows when they can actually do something.
  const showKeyboardHint =
    viewMode !== "translation" &&
    !recording.audioDeleted &&
    editedSegments.length > 0;

  // A general re-transcribe (force) for a recording that already has a timed transcript —
  // e.g. to redo it after switching Audio type to Music. The untimed case is handled by
  // `canEnablePerSentence`, so the two never both show.
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
          {/* Only offered with the filter on. "Mine all" while looking at the whole
              transcript reads as "mine everything", and the number beside it is the
              only thing that says otherwise. */}
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
                    ? `Make a card of every line shown. ${skippedWithinReach} skipped: already in your deck (marked "In deck"), or teaching a word another line here already covers.`
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
                // The box keeps focus so the next Enter steps again — the whole
                // point of a find bar is not having to click back into it.
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

      {/* Why the transcript below is empty, when the last run did not produce one. A
          cancel and a crash otherwise look identical to "never transcribed". */}
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
              {/* The same cancel the Library queue offers, so a long run started
                  here does not have to be abandoned by navigating away. It kills
                  the whisper process; the item resolves "cancelled" and the queue
                  moves on. */}
              {/* `ghost`, not the accent action style the Re-transcribe/Translate
                  buttons use in this same row: stopping something is not the
                  affirmative action, and it matches the Library queue's Cancel. */}
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
        // Sentences arriving from the running whisper pass. They replace the reading
        // panes for the duration: the transcript underneath is about to be overwritten
        // anyway, and watching it rebuild is the whole point. The reload-on-completion
        // effect above swaps the saved transcript back in the moment the run ends.
        //
        // Shown for the WHOLE run, not just once sentences exist — gating on a non-empty
        // list made the screen flip old transcript → live → skeleton → new transcript.
        // Its own waiting state covers the decode before the first sentence lands.
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
              minedKeys={minedKeys}
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
