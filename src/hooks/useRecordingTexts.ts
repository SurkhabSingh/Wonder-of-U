import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { RecordingTexts } from "../types";

export type RecordingTextsStatus = "idle" | "loading" | "error";

type UseRecordingTextsOptions = {
  filePath: string | null;
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
