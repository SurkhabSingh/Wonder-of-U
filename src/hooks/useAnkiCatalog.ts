import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { DEFAULT_ANKI_CATALOG } from "../constants";
import type { AnkiCatalog, BusyAction } from "../types";

type UseAnkiCatalogOptions = {
  noteType: string;
  persistSettingsIfNeeded: () => Promise<void>;
  setBusyAction: (busyAction: BusyAction) => void;
  setLoadError: (message: string) => void;
  showSuccess: (message: string) => void;
  showWarning: (message: string) => void;
};

export type RefreshAnkiCatalogOptions = {
  notifySuccess?: boolean;
  silent?: boolean;
  skipPersist?: boolean;
  suppressErrors?: boolean;
  // The note type a vocabulary row was just pointed at, passed explicitly for the
  // same reason `nextNoteType` is: the draft save is debounced, so its fields
  // would otherwise not be fetched until the save lands a beat later. The backend
  // adds this on top of the persisted rows rather than replacing them.
  vocabularyNoteType?: string;
};

export function useAnkiCatalog({
  noteType,
  persistSettingsIfNeeded,
  setBusyAction,
  setLoadError,
  showSuccess,
  showWarning,
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
          vocabularyNoteType: options?.vocabularyNoteType || null,
        });
        // The backend rebuilds the vocabulary field map from only the persisted
        // sources plus the one note type this call asked about, so a plain replace
        // would erase the fields of a row just pointed at a note type that is not
        // saved yet — the auto-refresh (or an Anki-offline reply, whose map is
        // empty) would wipe it a beat later. Field names for a note type do not
        // vanish once known, so carry earlier entries forward and let this call's
        // fresher answer win for the types it did fetch.
        setAnkiCatalog((previous) => ({
          ...catalog,
          vocabularyFieldMap: {
            ...previous.vocabularyFieldMap,
            ...catalog.vocabularyFieldMap,
          },
        }));
        if (catalog.status === "offline" && !options?.suppressErrors) {
          showWarning("Anki is offline currently.");
        } else if (catalog.status !== "offline" && options?.notifySuccess) {
          showSuccess("Anki refreshed.");
        }
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
    [
      noteType,
      persistSettingsIfNeeded,
      setBusyAction,
      setLoadError,
      showSuccess,
      showWarning,
    ],
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
