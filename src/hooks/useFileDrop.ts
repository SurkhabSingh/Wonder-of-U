import { useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { UnlistenFn } from "@tauri-apps/api/event";

type UseFileDropOptions = {
  // While disabled (an import is already running, say) the zone neither
  // highlights nor accepts a drop — the OS still delivers the event, we just
  // decline it rather than queueing a second import behind the first.
  enabled: boolean;
  onDrop: (paths: string[]) => void;
};

// File drag-and-drop for the Tauri webview.

export function useFileDrop({ enabled, onDrop }: UseFileDropOptions) {
  const [isDraggingOver, setIsDraggingOver] = useState(false);

  // The listener is registered exactly once, so it must not close over a stale
  // `onDrop`/`enabled`. Refs keep the handler current without re-subscribing.
  const onDropRef = useRef(onDrop);
  const enabledRef = useRef(enabled);

  useEffect(() => {
    onDropRef.current = onDrop;
  }, [onDrop]);

  useEffect(() => {
    enabledRef.current = enabled;
    if (!enabled) {
      setIsDraggingOver(false);
    }
  }, [enabled]);

  useEffect(() => {
    let active = true;
    let unlisten: UnlistenFn | null = null;

    void (async () => {
      const stop = await getCurrentWebview().onDragDropEvent((event) => {
        if (!active) {
          return;
        }

        const payload = event.payload;

        if (payload.type === "enter" || payload.type === "over") {
          setIsDraggingOver(enabledRef.current);
          return;
        }

        if (payload.type === "drop") {
          setIsDraggingOver(false);
          if (!enabledRef.current) {
            return;
          }
          onDropRef.current(payload.paths);
          return;
        }

        // "leave" — the pointer left the window without dropping.
        setIsDraggingOver(false);
      });

      if (!active) {
        // Unmounted while we were awaiting registration.
        stop();
        return;
      }

      unlisten = stop;
    })();

    return () => {
      active = false;
      unlisten?.();
      unlisten = null;
    };
  }, []);

  return { isDraggingOver };
}
