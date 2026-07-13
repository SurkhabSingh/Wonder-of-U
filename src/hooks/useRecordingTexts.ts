import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { RecordingTexts } from "../types";

export type RecordingTextsStatus = "idle" | "loading" | "error";

type UseRecordingTextsOptions = {
  filePath: string | null;
  // A cheap signature of anything that should force a re-read while the viewer
  // stays open (e.g. a fresh translation lands). Keeping this out of the fetch
  // itself lets a newly written sidecar appear without leaving the page.
  changeSignature: string;
};

export function useRecordingTexts({
  filePath,
  changeSignature,
}: UseRecordingTextsOptions) {
  const [data, setData] = useState<RecordingTexts | null>(null);
  const [status, setStatus] = useState<RecordingTextsStatus>(() =>
    filePath ? "loading" : "idle",
  );
  const [error, setError] = useState("");
  const [reloadToken, setReloadToken] = useState(0);

  const reload = useCallback(() => {
    setReloadToken((token) => token + 1);
  }, []);

  useEffect(() => {
    if (!filePath) {
      setData(null);
      setStatus("idle");
      setError("");
      return;
    }

    let cancelled = false;
    setStatus("loading");
    setError("");

    invoke<RecordingTexts>("read_recording_texts", { filePath })
      .then((result) => {
        if (cancelled) {
          return;
        }
        setData(result);
        setStatus("idle");
      })
      .catch((invokeError: unknown) => {
        if (cancelled) {
          return;
        }
        setData(null);
        setStatus("error");
        // A Tauri command returning `Result<_, String>` rejects with a plain
        // string, not an Error — handle that first so the specific backend
        // reason survives instead of always falling back to the generic text.
        setError(
          typeof invokeError === "string" && invokeError.trim()
            ? invokeError
            : invokeError instanceof Error && invokeError.message
              ? invokeError.message
              : "The transcript could not be loaded.",
        );
      });

    return () => {
      cancelled = true;
    };
  }, [filePath, changeSignature, reloadToken]);

  return { data, status, error, reload };
}
