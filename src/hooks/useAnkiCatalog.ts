import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { DEFAULT_ANKI_CATALOG } from "../constants";
import type { AnkiCatalog, BusyAction } from "../types";

type UseAnkiCatalogOptions = {
  noteType: string;
  persistSettingsIfNeeded: () => Promise<void>;
  setBusyAction: (busyAction: BusyAction) => void;
  setLoadError: (message: string) => void;
};

type RefreshAnkiCatalogOptions = {
  silent?: boolean;
  skipPersist?: boolean;
  suppressErrors?: boolean;
};

export function useAnkiCatalog({
  noteType,
  persistSettingsIfNeeded,
  setBusyAction,
  setLoadError,
}: UseAnkiCatalogOptions) {
  const [ankiCatalog, setAnkiCatalog] =
    useState<AnkiCatalog>(DEFAULT_ANKI_CATALOG);
  const autoRefreshInFlightRef = useRef(false);

  const refreshAnkiCatalog = useCallback(
    async (
      nextNoteType?: string,
      options?: RefreshAnkiCatalogOptions,
    ) => {
      try {
        if (!options?.silent) {
          setBusyAction("loadAnki");
        }
        if (!options?.skipPersist) {
          await persistSettingsIfNeeded();
        }
        const catalog = await invoke<AnkiCatalog>("load_anki_catalog", {
          noteType: (nextNoteType ?? noteType) || null,
        });
        setAnkiCatalog(catalog);
      } catch (error) {
        if (!options?.suppressErrors) {
          setLoadError(
            error instanceof Error
              ? error.message
              : "The Anki catalog could not be loaded.",
          );
        }
      } finally {
        if (!options?.silent) {
          setBusyAction(null);
        }
      }
    },
    [noteType, persistSettingsIfNeeded, setBusyAction, setLoadError],
  );

  useEffect(() => {
    let cancelled = false;

    async function refreshWhenAnkiIsAvailable() {
      if (cancelled || autoRefreshInFlightRef.current) {
        return;
      }

      autoRefreshInFlightRef.current = true;
      try {
        await refreshAnkiCatalog(undefined, {
          skipPersist: true,
          silent: true,
          suppressErrors: true,
        });
      } finally {
        autoRefreshInFlightRef.current = false;
      }
    }

    void refreshWhenAnkiIsAvailable();
    const interval = window.setInterval(refreshWhenAnkiIsAvailable, 10000);

    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [refreshAnkiCatalog]);

  return {
    ankiCatalog,
    refreshAnkiCatalog,
  };
}
