import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ScannableText } from "../components/watch/ScannableText";
import { LookupPopup } from "../components/watch/LookupPopup";
import { useWordScanner } from "../hooks/useWordScanner";
import type { AppBootstrap, WatchSnapshot } from "../types";
import { APP_SNAPSHOT_EVENT, DEFAULT_BOOTSTRAP } from "../constants";

// What the user sees over the video: the current subtitle line, and a dictionary popup
// when a word is hovered with the modifier held.
//
// The line comes from mpv's own `sub-text` / `sub-start` / `sub-end`, not from the parsed
// cue list — the spike confirmed those keep reporting with `sub-visibility=no`, so the
// overlay inherits mpv's timing and its `sub-delay` for free and cannot drift from what the
// player thinks is on screen.

const SCANNER_STATE_EVENT = "scanner-overlay-state";
/// Matches the watch page's poll. The overlay reads the same snapshot command, so this
/// costs mpv nothing it is not already paying.
const POLL_INTERVAL_MS = 250;

type ScannerState = {
  tracking: boolean;
  /// True while the modifier is held — pushed from Rust, because this window carries
  /// WS_EX_NOACTIVATE and so never receives key events of its own.
  scanning: boolean;
  width: number;
  height: number;
  dpi: number;
};

export function ScannerOverlay() {
  const [state, setState] = useState<ScannerState>({
    tracking: false,
    scanning: false,
    width: 0,
    height: 0,
    dpi: 96,
  });
  const [snapshot, setSnapshot] = useState<WatchSnapshot | null>(null);
  const [settings, setSettings] = useState(DEFAULT_BOOTSTRAP.settings);

  useEffect(() => {
    const unlisten = listen<ScannerState>(SCANNER_STATE_EVENT, (event) => {
      setState(event.payload);
    });
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, []);

  // Settings arrive the same way every other window gets them.
  useEffect(() => {
    let alive = true;
    void invoke<AppBootstrap>("get_app_bootstrap")
      .then((bootstrap) => {
        if (alive) {
          setSettings(bootstrap.settings);
        }
      })
      .catch(() => {
        // The defaults are already in state; a failed read is not worth a broken overlay.
      });
    const unlisten = listen<AppBootstrap>(APP_SNAPSHOT_EVENT, (event) => {
      setSettings(event.payload.settings);
    });
    return () => {
      alive = false;
      void unlisten.then((stop) => stop());
    };
  }, []);

  useEffect(() => {
    if (!state.tracking) {
      return;
    }
    let alive = true;
    const read = () => {
      void invoke<WatchSnapshot>("watch_snapshot")
        .then((next) => {
          if (alive) {
            setSnapshot(next);
          }
        })
        .catch(() => {
          if (alive) {
            setSnapshot(null);
          }
        });
    };
    read();
    const timer = window.setInterval(read, POLL_INTERVAL_MS);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [state.tracking]);

  // This window has its own document, so nothing has applied the theme or the reading
  // font to it — the main window's `useAppViewState` only reaches its own <html>.
  const theme = settings.theme === "light" ? "light" : "dark";
  const { readingFontFamily } = settings.scanner;
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    const root = document.documentElement.style;
    if (readingFontFamily.trim()) {
      root.setProperty("--font-reading", `${readingFontFamily}, var(--font-sans)`);
    } else {
      root.removeProperty("--font-reading");
    }
  }, [theme, readingFontFamily]);

  const scanner = useWordScanner({
    modifier: settings.scanner.modifier,
    releaseBehavior: settings.scanner.releaseBehavior,
    debounceMs: settings.scanner.debounceMs,
    // The authority for this window: Rust polls the key with GetAsyncKeyState, because a
    // window that never takes focus never sees a keydown.
    heldOverride: state.scanning,
    enabled: state.tracking,
  });

  const line = snapshot?.subtitleText?.trim() ?? "";
  // Keyed to the cue's start so a popup can never stay highlighted onto the next line.
  const ownerKey = `mpv:${snapshot?.subtitleStartMs ?? 0}`;

  // Click-through is a property of the whole window, so Rust has to know a popup is up or
  // releasing the modifier would make the entry unreadable the instant it appeared.
  //
  // The popup is deliberately NOT closed when the line changes: the video moving on does
  // not make a dictionary entry wrong, and yanking it away mid-sentence is worse than
  // letting it stand. The highlight goes on its own — its Range dies with the old text.
  const popupOpen = scanner.target !== null;
  useEffect(() => {
    void invoke("set_scanner_popup", { open: popupOpen });
  }, [popupOpen]);

  if (!state.tracking || !line) {
    return null;
  }

  return (
    <div className="scanner-root" data-theme={theme}>
      <div
        className={`scanner-line${state.scanning ? " is-scanning" : ""}`}
        style={{ fontSize: `${settings.scanner.overlayFontSizePx}px` }}
      >
        <ScannableText ownerKey={ownerKey} text={line} />
      </div>

      {scanner.target ? (
        <LookupPopup
          anchor={scanner.target.anchor}
          result={scanner.result}
          isLoading={scanner.isLoading}
          error={scanner.error}
          theme={theme}
          fontFamily={settings.scanner.fontFamily}
          fontSizePx={settings.scanner.fontSizePx}
          onClose={scanner.close}
        />
      ) : null}
    </div>
  );
}
