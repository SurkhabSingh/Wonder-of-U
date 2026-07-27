/* Vendored from the user's Anki add-on (`anki_lookup_dev/web/scanner-core.js`).
 *
 * The add-on already ships this as a UMD module with zero DOM access, zero Anki coupling
 * and no `pycmd` — it is the pure half of its popup: word segmentation, sentence and cloze
 * extraction, pitch morae and levels, popup geometry, and the lookup throttle. Reusing it
 * verbatim is what keeps the app's scanner and the add-on's from drifting on the details
 * that are easy to get subtly wrong (small-kana mora merging, sentence sanitisation, the
 * "time since last lookup" debounce floor).
 *
 * Vendored rather than imported from the add-on directory so the app builds and runs with
 * Anki absent. Body below is byte-for-byte the add-on's; only this header and the ESM
 * export at the foot are added.
 */
(function (root, factory) {
    const api = factory();
    if (typeof module === "object" && module.exports) {
        module.exports = api;
    }
    root.AnkiLookupScannerCore = api;
})(typeof globalThis !== "undefined" ? globalThis : this, function () {
    "use strict";

    const WORD_PATTERN = /[\p{L}\p{N}\p{M}_'’\-]/u;

    const JAPANESE_CHARACTER_PATTERN =
        /[\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Han}々〆ヶー]/u;
    const SMALL_KANA_PATTERN = /[ぁぃぅぇぉゃゅょゎァィゥェォャュョヮ]/u;
    // Han only, plus the iteration mark 々 which stands in for a repeated kanji. Not
    // the same as JAPANESE_CHARACTER_PATTERN, which also matches kana: only a kanji
    // has a kanji entry to open, so kana in a headword must stay unclickable.
    const KANJI_PATTERN = /[\p{Script=Han}々]/u;
    const segmenters = new Map();

    function getSegmenter(locale, granularity) {
        if (typeof Intl === "undefined" || typeof Intl.Segmenter !== "function") {
            return null;
        }
        const key = `${locale || ""}:${granularity}`;
        if (!segmenters.has(key)) {
            segmenters.set(
                key,
                new Intl.Segmenter(locale || undefined, { granularity }),
            );
        }
        return segmenters.get(key);
    }

    function normalizeTerm(value, maximumLength) {
        return String(value || "")
            .replace(/\s+/gu, " ")
            .trim()
            .slice(0, maximumLength);
    }

    function sanitizeSentence(value, maximumLength = 2000) {
        let sentence = normalizeTerm(value, maximumLength);
        const openingBraces = (sentence.match(/\{/gu) || []).length;
        const closingBraces = (sentence.match(/\}/gu) || []).length;
        if (openingBraces > closingBraces) {
            sentence = sentence.replace(/^\{+\s*/u, "");
        } else if (closingBraces > openingBraces) {
            sentence = sentence.replace(/^\}+\s*/u, "");
        }

        const remainingOpeningBraces = (sentence.match(/\{/gu) || []).length;
        const remainingClosingBraces = (sentence.match(/\}/gu) || []).length;
        if (remainingOpeningBraces > remainingClosingBraces) {
            sentence = sentence.replace(/\s*\{+$/u, "");
        } else if (remainingClosingBraces > remainingOpeningBraces) {
            sentence = sentence.replace(/\s*\}+$/u, "");
        }
        return sentence.trim();
    }

    function isKanji(character) {
        return typeof character === "string" && KANJI_PATTERN.test(character);
    }

    function japaneseCandidates(text, start, maximumLength) {
        if (!text || start < 0 || start >= text.length) {
            return [];
        }
        const run = [];
        for (const character of Array.from(text.slice(start))) {
            if (!JAPANESE_CHARACTER_PATTERN.test(character)) {
                break;
            }
            run.push(character);
            if (run.length >= maximumLength) {
                break;
            }
        }
        const candidates = [];
        for (let length = run.length; length > 0; length -= 1) {
            candidates.push(run.slice(0, length).join(""));
        }
        return candidates;
    }

    function lookupCandidates(text, start, initialTerm, maximumLength, locale) {
        if (!text || start < 0 || start >= text.length) {
            return initialTerm ? [initialTerm] : [];
        }
        if (JAPANESE_CHARACTER_PATTERN.test(text[start])) {
            return japaneseCandidates(text, start, Math.min(maximumLength, 20));
        }

        const endLimit = Math.min(text.length, start + maximumLength);
        const source = text.slice(start, endLimit);
        const candidates = [];
        const segmenter = getSegmenter(locale, "word");
        if (segmenter) {
            let wordCount = 0;
            for (const part of segmenter.segment(source)) {
                if (/[\n.!?。！？]/u.test(part.segment)) {
                    break;
                }
                if (!part.isWordLike) {
                    continue;
                }
                wordCount += 1;
                candidates.push(normalizeTerm(source.slice(0, part.index + part.segment.length), maximumLength));
                if (wordCount >= 6) {
                    break;
                }
            }
        }
        if (!candidates.length && initialTerm) {
            candidates.push(normalizeTerm(initialTerm, maximumLength));
        }
        return [...new Set(candidates.filter(Boolean))].reverse();
    }

    function segmentAt(text, offset, locale) {
        if (!text || offset < 0 || offset > text.length) {
            return null;
        }

        const segmenter = getSegmenter(locale, "word");
        if (segmenter) {
            for (const part of segmenter.segment(text)) {
                const end = part.index + part.segment.length;
                if (
                    part.isWordLike &&
                    offset >= part.index &&
                    offset <= end
                ) {
                    return {
                        term: part.segment,
                        start: part.index,
                        end,
                    };
                }
            }
        }

        let probe = Math.min(offset, text.length - 1);
        if (probe >= 0 && !WORD_PATTERN.test(text[probe]) && probe > 0) {
            probe -= 1;
        }
        if (probe < 0 || !WORD_PATTERN.test(text[probe])) {
            return null;
        }

        let start = probe;
        let end = probe + 1;
        while (start > 0 && WORD_PATTERN.test(text[start - 1])) {
            start -= 1;
        }
        while (end < text.length && WORD_PATTERN.test(text[end])) {
            end += 1;
        }

        return { term: text.slice(start, end), start, end };
    }

    function matchesShortcut(event, shortcut) {
        const parts = String(shortcut || "")
            .split("+")
            .map((part) => part.trim().toLowerCase())
            .filter(Boolean);
        if (!parts.length) {
            return false;
        }

        const key = parts[parts.length - 1];
        return (
            event.key.toLowerCase() === key &&
            event.ctrlKey === parts.includes("ctrl") &&
            event.shiftKey === parts.includes("shift") &&
            event.altKey === parts.includes("alt") &&
            event.metaKey === parts.includes("meta")
        );
    }

    function clampPopupSize(width, height, viewportWidth, viewportHeight, margin) {
        const availableWidth = Math.max(240, viewportWidth - margin * 2);
        const availableHeight = Math.max(180, viewportHeight - margin * 2);
        return {
            width: Math.min(availableWidth, Math.max(280, width)),
            height: Math.min(availableHeight, Math.max(220, height)),
        };
    }

    function lookupDelay(now, previousStart, interval) {
        return Math.max(0, interval - (now - previousStart));
    }

    function canOpenNestedPopup(sourceDepth, enabled, maximumDepth) {
        return enabled && sourceDepth + 1 < maximumDepth;
    }

    function japaneseMorae(reading) {
        const morae = [];
        for (const character of Array.from(reading || "")) {
            if (SMALL_KANA_PATTERN.test(character) && morae.length) {
                morae[morae.length - 1] += character;
            } else {
                morae.push(character);
            }
        }
        return morae;
    }

    function pitchLevels(moraCount, position) {
        const count = Math.max(0, moraCount);
        if (typeof position === "string" && /^[HL]+$/u.test(position)) {
            const values = Array.from(position, (level) => level === "H");
            while (values.length < count + 1) {
                values.push(values[values.length - 1] || false);
            }
            return values.slice(0, count + 1);
        }
        const downstep =
            Number.isInteger(position) && position >= 0 ? position : 0;
        const levels = [];
        for (let index = 0; index <= count; index += 1) {
            if (downstep === 0) {
                levels.push(index > 0);
            } else if (downstep === 1) {
                levels.push(index === 0);
            } else {
                levels.push(index > 0 && index < downstep);
            }
        }
        return levels;
    }

    function isPopupDescendant(state, ancestor) {
        let parent = state.parent;
        while (parent) {
            if (parent === ancestor) {
                return true;
            }
            parent = parent.parent;
        }
        return false;
    }

    /**
     * Place a popup relative to the text it describes.
     *
     * `userSized` is the difference between "how tall does this fit?" and "how tall
     * did the user make it?". By default the height is capped to the room beside the
     * anchor, which is right for an automatic opening. Once the user has resized the
     * box that cap becomes a bug: it silently overrode their drag, the box refused to
     * grow, and pinning was the only way out. When they have sized it, the popup still
     * follows the word and still prefers the side with room — but it keeps its height
     * and will cover the word rather than shrink.
     */
    function anchoredVerticalPosition(
        anchor,
        requestedHeight,
        viewportHeight,
        margin,
        gap,
        userSized = false,
    ) {
        const viewportHeightAvailable = Math.max(0, viewportHeight - margin * 2);
        if (!anchor) {
            return {
                top: margin,
                height: Math.min(requestedHeight, viewportHeightAvailable),
                placement: "below",
            };
        }

        const belowTop = Math.max(
            margin,
            Math.min(anchor.bottom + gap, viewportHeight - margin),
        );
        const aboveBottom = Math.max(
            margin,
            Math.min(anchor.top - gap, viewportHeight - margin),
        );
        const availableBelow = Math.max(
            0,
            viewportHeight - margin - belowTop,
        );
        const availableAbove = Math.max(0, aboveBottom - margin);
        const placement =
            availableBelow >= requestedHeight || availableBelow >= availableAbove
                ? "below"
                : "above";

        if (!userSized) {
            const availableHeight =
                placement === "above" ? availableAbove : availableBelow;
            const height = Math.min(requestedHeight, availableHeight);
            return {
                top: placement === "above" ? aboveBottom - height : belowTop,
                height,
                placement,
            };
        }

        const height = Math.min(requestedHeight, viewportHeightAvailable);
        const preferredTop = placement === "above" ? aboveBottom - height : belowTop;
        const top = Math.max(
            margin,
            Math.min(preferredTop, viewportHeight - margin - height),
        );
        return { top, height, placement };
    }

    function popupPosition(
        anchor,
        size,
        viewportWidth,
        viewportHeight,
        margin,
        gap,
        userSized = false,
    ) {
        const preferredLeft = anchor ? anchor.left : (viewportWidth - size.width) / 2;
        const vertical = anchoredVerticalPosition(
            anchor,
            size.height,
            viewportHeight,
            margin,
            gap,
            userSized,
        );
        return {
            left: Math.max(
                margin,
                Math.min(preferredLeft, viewportWidth - size.width - margin),
            ),
            ...vertical,
        };
    }

    function nestedPopupPosition(
        parent,
        anchor,
        size,
        viewportWidth,
        viewportHeight,
        margin,
        gap,
        userSized = false,
    ) {
        const right = parent.right + gap;
        const left = parent.left - size.width - gap;
        const preferredLeft =
            right + size.width <= viewportWidth - margin
                ? right
                : left >= margin
                  ? left
                  : anchor.left;
        const vertical = anchoredVerticalPosition(
            anchor,
            size.height,
            viewportHeight,
            margin,
            gap,
            userSized,
        );
        return {
            left: Math.max(
                margin,
                Math.min(preferredLeft, viewportWidth - size.width - margin),
            ),
            ...vertical,
        };
    }

    function sourceRailPlacement(
        popupLeft,
        popupWidth,
        viewportWidth,
        margin,
        railWidth,
        gap,
        preferRight,
    ) {
        const assemblyWidth = popupWidth + railWidth + gap;
        if (assemblyWidth > viewportWidth - margin * 2) {
            return {
                popupLeft: Math.max(
                    margin,
                    Math.min(popupLeft, viewportWidth - popupWidth - margin),
                ),
                side: "inside",
            };
        }
        const canUseLeft = popupLeft - railWidth - gap >= margin;
        const canUseRight =
            popupLeft + popupWidth + railWidth + gap <= viewportWidth - margin;
        if (preferRight && canUseRight) {
            return { popupLeft, side: "right" };
        }
        if (!preferRight && canUseLeft) {
            return { popupLeft, side: "left" };
        }
        if (canUseLeft) {
            return { popupLeft, side: "left" };
        }
        if (canUseRight) {
            return { popupLeft, side: "right" };
        }
        return preferRight
            ? {
                  popupLeft: viewportWidth - margin - assemblyWidth,
                  side: "right",
              }
            : {
                  popupLeft: margin + railWidth + gap,
                  side: "left",
              };
    }

    function clampDraggedPopupPosition(
        left,
        top,
        size,
        viewportWidth,
        viewportHeight,
        margin,
        railSide,
        railWidth,
        gap,
    ) {
        const minimumLeft =
            railSide === "left" ? margin + railWidth + gap : margin;
        const maximumLeft =
            viewportWidth -
            margin -
            size.width -
            (railSide === "right" ? railWidth + gap : 0);
        return {
            left: Math.max(minimumLeft, Math.min(left, maximumLeft)),
            top: Math.max(
                margin,
                Math.min(top, viewportHeight - margin - size.height),
            ),
        };
    }

    function sentenceRangeAt(text, offset, locale) {
        const segmenter = getSegmenter(locale, "sentence");
        if (segmenter) {
            for (const part of segmenter.segment(text)) {
                const end = part.index + part.segment.length;
                if (offset >= part.index && offset <= end) {
                    return { start: part.index, end };
                }
            }
        }

        const boundary = /[.!?。！？\n]/u;
        let start = Math.min(offset, text.length);
        let end = start;
        while (start > 0 && !boundary.test(text[start - 1])) {
            start -= 1;
        }
        while (end < text.length && !boundary.test(text[end])) {
            end += 1;
        }
        if (end < text.length) {
            end += 1;
        }
        return { start, end };
    }

    function sentenceAt(text, offset, locale) {
        if (!text || offset < 0 || offset > text.length) {
            return "";
        }
        const range = sentenceRangeAt(text, offset, locale);
        return sanitizeSentence(text.slice(range.start, range.end));
    }

    /**
     * The sentence around the scan point, plus where the scanned word sits in it.
     *
     * The offset is found by locating `term` in the *sanitised* sentence rather than
     * by carrying the original index through sanitisation. Sanitising trims, collapses
     * whitespace and strips unbalanced braces, each of which shifts every index after
     * it; replaying all that to keep an offset in step would be a second implementation
     * of the same transforms, and would drift the moment either changed. Searching for
     * the word cannot drift, because it reads the string we actually send.
     *
     * The original position still matters when a word appears more than once — 「パンを
     * 食べる前にパンを見た」 — so it is used to pick between occurrences, not to index.
     *
     * Returns an offset in **UTF-16 code units**, which is what JavaScript counts;
     * Python converts to codepoints before slicing.
     */
    function sentenceContextAt(text, offset, locale, term) {
        const sentence = sentenceAt(text, offset, locale);
        if (!sentence || !term) {
            return { text: sentence, offset: 0, term: "" };
        }

        const range = sentenceRangeAt(text, offset, locale);
        const hint = Math.max(0, offset - range.start);
        const index = nearestIndexOf(sentence, term, hint);
        if (index < 0) {
            // The scanned form is not in the sentence we are about to send — the
            // segmenter and the scanner disagreed about a boundary. Better an empty
            // cloze body than one cut in the wrong place; {sentence} still works.
            return { text: sentence, offset: 0, term: "" };
        }
        return { text: sentence, offset: index, term };
    }

    /** The occurrence of `needle` nearest `hint`, or -1. */
    function nearestIndexOf(haystack, needle, hint) {
        let best = -1;
        let bestDistance = Infinity;
        let index = haystack.indexOf(needle);
        while (index !== -1) {
            const distance = Math.abs(index - hint);
            if (distance < bestDistance) {
                best = index;
                bestDistance = distance;
            }
            index = haystack.indexOf(needle, index + 1);
        }
        return best;
    }

    /* Translation ------------------------------------------------------------
       The Python half of this lives in translation/external.py. Both are driven by
       tests/fixtures/external_urls.json so they cannot drift apart. */

    const GOOGLE_TRANSLATE = "google-translate";
    const DEEPL = "deepl";
    const MAX_EXTERNAL_TEXT_LENGTH = 1800;

    const PROVIDER_LABELS = {
        [GOOGLE_TRANSLATE]: "Google Translate",
        [DEEPL]: "DeepL",
    };

    function providerLabel(provider) {
        return PROVIDER_LABELS[provider] || provider;
    }

    function truncateForExternalUrl(text) {
        return String(text || "").slice(0, MAX_EXTERNAL_TEXT_LENGTH);
    }

    /* Translation state ------------------------------------------------------
       The reducer the popup renders from. Kept pure and here rather than in
       popup.js so it can be tested: popup.js is DOM code with no test harness. */

    const TRANSLATION_IDLE = "idle";
    const TRANSLATION_PENDING = "pending";
    const TRANSLATION_READY = "ready";
    const TRANSLATION_UNAVAILABLE = "unavailable";
    const TRANSLATION_ERROR = "error";

    /**
     * Decide what a translation panel should show.
     *
     * `result` is the payload from Python, or null when the tab has never been
     * activated. Returns the state name, the text to show, and which actions the
     * panel offers — the panel renders exactly this and decides nothing itself.
     */
    function translationState(result) {
        if (!result) {
            return { state: TRANSLATION_IDLE, message: "", text: "", actions: [] };
        }

        if (result.status === TRANSLATION_PENDING) {
            return {
                state: TRANSLATION_PENDING,
                message: `Translating with ${providerLabel(result.provider)}...`,
                text: "",
                actions: ["cancel"],
            };
        }

        if (result.status === TRANSLATION_READY) {
            return {
                state: TRANSLATION_READY,
                message: "",
                text: result.text || "",
                cached: Boolean(result.cached),
                actions: ["copy", "retry"],
            };
        }

        if (result.status === TRANSLATION_UNAVAILABLE) {
            return {
                state: TRANSLATION_UNAVAILABLE,
                message: result.message || "Translation is unavailable.",
                text: "",
                actions: result.external_url ? ["open_external"] : [],
            };
        }

        return {
            state: TRANSLATION_ERROR,
            message: result.message || "The translation failed.",
            text: "",
            actions: result.external_url ? ["retry", "open_external"] : ["retry"],
        };
    }

    /**
     * Whether activating a tab should start a translation.
     *
     * Lazy on purpose. Hold-to-scan fires a lookup on every pointer move; if each
     * one also queued a translation, a few seconds of scanning would fill a 64-deep
     * queue against an extension that translates one job at a time.
     */
    function shouldRequestTranslation(tabState) {
        if (!tabState || !tabState.sentence) {
            return false;
        }
        if (tabState.requested) {
            return false;
        }
        return true;
    }

    /** Whether a pushed result belongs to the popup and request that is showing. */
    function isCurrentTranslation(payload, popupToken, requestId) {
        if (!payload || payload.kind !== "translation") {
            return false;
        }
        return payload.popup_token === popupToken && payload.request_id === requestId;
    }

    /* Resize -------------------------------------------------------------------
       Four corner grips. Each grows along its own diagonal and keeps the opposite
       corner still, which is what every resizable window does and what makes the
       gesture predictable regardless of where the popup happens to sit.

       Automatic placement decides where a popup OPENS — below the scanned word, or
       above it when there is more room there. But once the user drags a grip, the
       size they asked for wins: the popup is theirs, and it may cover the word if
       that is what they wanted. Before this, an unpinned popup's height was silently
       overridden by whatever space happened to be left beside the anchor, so the box
       refused to grow and the only escape was to pin it. */

    const RESIZE_CORNERS = ["top-left", "top-right", "bottom-left", "bottom-right"];

    function isResizeCorner(corner) {
        return RESIZE_CORNERS.indexOf(corner) !== -1;
    }

    /** The size a drag is asking for, before clamping. */
    function resizeDelta(corner, startRect, dx, dy) {
        const growsLeft = corner === "top-left" || corner === "bottom-left";
        const growsUp = corner === "top-left" || corner === "top-right";
        return {
            width: startRect.width + (growsLeft ? -dx : dx),
            height: startRect.height + (growsUp ? -dy : dy),
        };
    }

    /**
     * Where the box lands once its size is settled.
     *
     * Derived from the *opposite* edge rather than by accumulating the pointer delta,
     * so a size that got clamped cannot drag the anchored corner out of place: the
     * grip stops moving, the far corner stays exactly where it was.
     */
    function resizeGeometry(corner, startRect, size) {
        const growsLeft = corner === "top-left" || corner === "bottom-left";
        const growsUp = corner === "top-left" || corner === "top-right";
        return {
            left: growsLeft ? startRect.right - size.width : startRect.left,
            top: growsUp ? startRect.bottom - size.height : startRect.top,
            width: size.width,
            height: size.height,
        };
    }

    /* Source rail keyboard navigation ------------------------------------------
       The rail is an ARIA tablist, and a tablist without arrow keys is not one: the
       pattern requires Left/Right (or Up/Down for a vertical rail) to move between
       tabs, Home/End to jump to the ends, and wrapping at both edges. */

    const TAB_NAVIGATION_KEYS = new Set([
        "ArrowRight",
        "ArrowLeft",
        "ArrowDown",
        "ArrowUp",
        "Home",
        "End",
    ]);

    function isTabNavigationKey(key) {
        return TAB_NAVIGATION_KEYS.has(key);
    }

    /**
     * Return the tab index a navigation key should move to.
     *
     * Returns the current index unchanged for anything that is not a navigation key,
     * so the caller can use the result to decide whether to preventDefault.
     */
    function nextTabIndex(current, count, key) {
        if (count <= 0) {
            return 0;
        }
        const index = Math.min(Math.max(current, 0), count - 1);

        if (key === "Home") {
            return 0;
        }
        if (key === "End") {
            return count - 1;
        }
        if (key === "ArrowRight" || key === "ArrowDown") {
            return (index + 1) % count;
        }
        if (key === "ArrowLeft" || key === "ArrowUp") {
            return (index - 1 + count) % count;
        }
        return index;
    }

    /* Note creation ----------------------------------------------------------- */

    /**
     * Decide what the Add control should say and whether it is usable.
     *
     * `result` is the payload from Python, or null before anything was pressed.
     * `configured` comes from the injected config, so an unconfigured preset disables
     * the button up front rather than failing after the user commits to it mid-review.
     */
    function noteState(result, configured) {
        if (!configured) {
            return {
                state: "not_configured",
                label: "Add note",
                message: "Configure a note preset in Anki Lookup: Settings.",
                enabled: false,
                actions: [],
            };
        }

        if (!result) {
            return { state: "idle", label: "Add note", message: "", enabled: true, actions: [] };
        }

        if (result.status === "queued") {
            return { state: "queued", label: "Adding...", message: "", enabled: false, actions: [] };
        }

        if (result.status === "added") {
            return {
                state: "added",
                label: "Added",
                message: "Note added.",
                enabled: false,
                actions: ["open_note"],
            };
        }

        if (result.status === "duplicate") {
            return {
                state: "duplicate",
                label: "Add note",
                message: result.message || "This note already exists.",
                enabled: true,
                actions: ["open_note", "add_anyway"],
            };
        }

        if (result.status === "not_configured") {
            return {
                state: "not_configured",
                label: "Add note",
                message: result.message || "Configure a note preset first.",
                enabled: false,
                actions: [],
            };
        }

        return {
            state: "error",
            label: "Add note",
            message: result.message || "The note could not be created.",
            enabled: true,
            actions: [],
        };
    }

    /** Whether a pushed note result belongs to the popup and request showing. */
    function isCurrentNote(payload, popupToken, requestId) {
        if (!payload || payload.kind !== "note") {
            return false;
        }
        return payload.popup_token === popupToken && payload.request_id === requestId;
    }

    function externalTranslateUrl(provider, text, sourceLang, targetLang) {
        const trimmed = truncateForExternalUrl(text);
        const source = sourceLang || "auto";
        const target = targetLang || "en";

        if (provider === DEEPL) {
            // DeepL carries the text in the fragment, not the query.
            return (
                "https://www.deepl.com/translator#" +
                `${encodeURIComponent(source)}/${encodeURIComponent(target)}/` +
                encodeURIComponent(trimmed)
            );
        }

        const query = new URLSearchParams();
        query.set("sl", source);
        query.set("tl", target);
        query.set("text", trimmed);
        query.set("op", "translate");
        return `https://translate.google.com/?${query.toString()}`;
    }

    return {
        clampPopupSize,
        clampDraggedPopupPosition,
        canOpenNestedPopup,
        externalTranslateUrl,
        isCurrentNote,
        isCurrentTranslation,
        isPopupDescendant,
        isResizeCorner,
        isKanji,
        isTabNavigationKey,
        japaneseMorae,
        japaneseCandidates,
        lookupCandidates,
        lookupDelay,
        matchesShortcut,
        nestedPopupPosition,
        nextTabIndex,
        normalizeTerm,
        noteState,
        pitchLevels,
        popupPosition,
        providerLabel,
        resizeDelta,
        resizeGeometry,
        sanitizeSentence,
        segmentAt,
        sentenceAt,
        sentenceContextAt,
        shouldRequestTranslation,
        sourceRailPlacement,
        translationState,
        truncateForExternalUrl,
    };
});

// The UMD wrapper above assigns the API to `globalThis` (and to `module.exports`, which is
// simply absent under ESM — the `typeof` guard makes that a no-op, not an error). This line
// gives the bundler a real entry point instead of relying on a global.
export default globalThis.AnkiLookupScannerCore;
