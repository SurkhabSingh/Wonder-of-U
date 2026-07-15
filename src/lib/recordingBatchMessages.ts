import type { RecordingBatchResult } from "../types";

export type RecordingBatchAction =
  | "transcribe"
  | "translate"
  | "delete"
  | "anki"
  | "furigana"
  | "convert"
  | "import";

export function formatBatchToastMessage(
  action: RecordingBatchAction,
  result: RecordingBatchResult,
): string {
  const successCount = result.items.filter((item) => item.status === "success").length;
  const skippedCount = result.items.filter((item) => item.status === "skipped").length;
  const failedItems = result.items.filter((item) => item.status === "failed");
  const failedCount = failedItems.length;
  const firstFailure = failedItems[0]?.message;
  const furiganaSkippedCount = result.items.filter((item) =>
    item.message.toLowerCase().includes("furigana was skipped"),
  ).length;

  if (action === "anki") {
    if (failedCount > 0 && successCount === 0) {
      return firstFailure
        ? `No cards were pushed to Anki. ${firstFailure}`
        : "No cards were pushed to Anki.";
    }

    if (failedCount > 0) {
      return `${successCount} card${successCount === 1 ? "" : "s"} pushed to Anki. ${failedCount} failed: ${firstFailure ?? "check the saved recordings list."}`;
    }

    if (successCount === 0 && skippedCount > 0) {
      return `${skippedCount} card${skippedCount === 1 ? " is" : "s are"} already in the selected Anki deck.`;
    }

    const baseMessage = `${successCount} card${successCount === 1 ? "" : "s"} pushed to Anki.`;
    return furiganaSkippedCount > 0
      ? `${baseMessage} Furigana was skipped for ${furiganaSkippedCount} because the Anki Lookup add-on was unavailable.`
      : baseMessage;
  }

  if (action === "furigana") {
    if (failedCount > 0 && successCount === 0) {
      return firstFailure
        ? `No Anki cards were updated with furigana. ${firstFailure}`
        : "No Anki cards were updated with furigana.";
    }

    if (failedCount > 0) {
      return `${successCount} Anki card${successCount === 1 ? "" : "s"} updated with furigana. ${failedCount} failed: ${firstFailure ?? "check the saved recordings list."}`;
    }

    return `${successCount} Anki card${successCount === 1 ? "" : "s"} updated with furigana.`;
  }

  if (action === "convert") {
    if (failedCount > 0 && successCount === 0) {
      return firstFailure ?? "No recordings were converted to MP3.";
    }

    if (failedCount > 0) {
      return `${successCount} recording${successCount === 1 ? "" : "s"} converted to MP3. ${failedCount} failed.`;
    }

    if (successCount === 0 && skippedCount > 0) {
      return `${skippedCount} recording${skippedCount === 1 ? " was" : "s were"} skipped. Only transcribed WAV recordings can be converted.`;
    }

    return `${successCount} recording${successCount === 1 ? "" : "s"} converted to MP3.`;
  }

  // Import deliberately does not transcribe, so a successful import is only a
  // half-finished job — say where the files went and what is left to do. A file
  // that needed ffmpeg and did not find it fails on its own without taking the
  // rest of the batch down, so a partial result must name that failure.
  if (action === "import") {
    if (failedCount > 0 && successCount === 0) {
      return firstFailure ?? "No files were imported.";
    }

    if (failedCount > 0) {
      return `${successCount} file${successCount === 1 ? "" : "s"} imported. ${failedCount} failed: ${firstFailure ?? "check the file format."}`;
    }

    if (successCount === 0) {
      return result.message;
    }

    return `${successCount} file${successCount === 1 ? "" : "s"} imported. Transcribe from the library when you are ready.`;
  }

  if (failedCount > 0 && successCount === 0) {
    return firstFailure ?? result.message;
  }

  return result.message;
}
