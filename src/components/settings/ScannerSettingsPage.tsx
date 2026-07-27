import { ThemedSelect } from "../ui/ThemedSelect";
import { TooltipBadge } from "../ui/Tooltip";
import {
  FONT_FAMILY_OPTIONS,
  POPUP_FONT_SIZE_OPTIONS,
  READING_FONT_SIZE_OPTIONS,
  SCAN_DEBOUNCE_OPTIONS,
  SCAN_MODIFIER_OPTIONS,
  SCAN_RELEASE_OPTIONS,
  SUBTITLE_FONT_SIZE_OPTIONS,
} from "../../constants";
import type { AppSettings } from "../../types";
import type { SettingsUpdate } from "./settingsTypes";

/// Surfaces a hand-edited value that is not one of the presets, rather than silently
/// showing the wrong entry. Same idiom as the clip-padding and deck pickers.
function withCurrent(
  options: readonly { value: string; label: string }[],
  current: string,
  label: (value: string) => string,
) {
  return options.some((option) => option.value === current)
    ? [...options]
    : [{ value: current, label: label(current) }, ...options];
}

export function ScannerSettingsPage({
  settingsDraft,
  onUpdateSettings,
}: {
  settingsDraft: AppSettings;
  onUpdateSettings: (update: SettingsUpdate) => void;
}) {
  const scanner = settingsDraft.scanner;

  return (
    <>
      <header className="panel-header">
        <div>
          <p className="panel-kicker">Settings</p>
          <h2>Dictionary &amp; Reading</h2>
        </div>
      </header>

      <div className="info-note">
        <p className="microcopy">
          Hold a key and hover a word to see its dictionary entry &mdash; in the subtitle
          list, and over the video when the overlay is on. The dictionary is your{" "}
          <strong>Anki add-on&rsquo;s</strong>, so <strong>Anki has to be running</strong>;
          mining already needs it, so this asks for nothing new.
        </p>
      </div>

      <div className="settings-grid">
        <label className="field">
          <span>
            Scan with
            <TooltipBadge
              label="?"
              description="The key you hold while hovering a word. Over the video this key does double duty: while it is down the overlay takes the mouse, and the moment you release it every click goes back to mpv."
            />
          </span>
          <ThemedSelect
            value={scanner.modifier}
            options={[...SCAN_MODIFIER_OPTIONS]}
            placeholder="Scan modifier"
            onChange={(value) =>
              onUpdateSettings({
                scanner: { modifier: value as typeof scanner.modifier },
              })
            }
          />
        </label>

        <label className="field">
          <span>
            When you let go
            <TooltipBadge
              label="?"
              description="Whether releasing the key closes the popup or leaves it up so you can read and scroll it."
            />
          </span>
          <ThemedSelect
            value={scanner.releaseBehavior}
            options={[...SCAN_RELEASE_OPTIONS]}
            placeholder="On release"
            onChange={(value) =>
              onUpdateSettings({
                scanner: {
                  releaseBehavior: value as typeof scanner.releaseBehavior,
                },
              })
            }
          />
        </label>

        <label className="field">
          <span>
            Lookup delay
            <TooltipBadge
              label="?"
              description="A floor on how often a lookup may start, not a wait before the first one. At the default the underline follows your pointer instantly and only the dictionary query is paced."
            />
          </span>
          <ThemedSelect
            value={String(scanner.debounceMs)}
            options={withCurrent(
              SCAN_DEBOUNCE_OPTIONS,
              String(scanner.debounceMs),
              (value) => `${value} ms`,
            )}
            placeholder="Delay"
            onChange={(value) =>
              onUpdateSettings({ scanner: { debounceMs: Number(value) } })
            }
          />
        </label>

        <label className="field">
          <span>Popup font</span>
          <ThemedSelect
            value={scanner.fontFamily}
            options={[...FONT_FAMILY_OPTIONS]}
            placeholder="Match the app"
            onChange={(value) =>
              onUpdateSettings({ scanner: { fontFamily: value } })
            }
          />
        </label>

        <label className="field">
          <span>
            Popup text size
            <TooltipBadge
              label="?"
              description="The whole popup is sized relative to this one value, so it scales as a unit rather than only changing the definitions."
            />
          </span>
          <ThemedSelect
            value={String(scanner.fontSizePx)}
            options={withCurrent(
              POPUP_FONT_SIZE_OPTIONS,
              String(scanner.fontSizePx),
              (value) => `${value} px`,
            )}
            placeholder="Popup size"
            onChange={(value) =>
              onUpdateSettings({ scanner: { fontSizePx: Number(value) } })
            }
          />
        </label>

        <label className="field">
          <span>
            Subtitles over the video
            <TooltipBadge
              label="?"
              description="Text size for the scannable subtitle line drawn over mpv. Only used while the overlay is switched on from the Watch page."
            />
          </span>
          <ThemedSelect
            value={String(scanner.overlayFontSizePx)}
            options={withCurrent(
              SUBTITLE_FONT_SIZE_OPTIONS,
              String(scanner.overlayFontSizePx),
              (value) => `${value} px`,
            )}
            placeholder="Subtitle size"
            onChange={(value) =>
              onUpdateSettings({ scanner: { overlayFontSizePx: Number(value) } })
            }
          />
        </label>

        <label className="field">
          <span>
            Reading font
            <TooltipBadge
              label="?"
              description="Used for transcripts, the live transcript and the subtitle line. Your choice is placed ahead of the built-in stack rather than replacing it, so a font without Japanese glyphs still falls back instead of showing boxes."
            />
          </span>
          <ThemedSelect
            value={scanner.readingFontFamily}
            options={[...FONT_FAMILY_OPTIONS]}
            placeholder="Built-in"
            onChange={(value) =>
              onUpdateSettings({ scanner: { readingFontFamily: value } })
            }
          />
        </label>

        <label className="field">
          <span>Reading text size</span>
          <ThemedSelect
            value={String(scanner.readingFontSizePx)}
            options={withCurrent(
              READING_FONT_SIZE_OPTIONS,
              String(scanner.readingFontSizePx),
              (value) => `${value} px`,
            )}
            placeholder="Reading size"
            onChange={(value) =>
              onUpdateSettings({ scanner: { readingFontSizePx: Number(value) } })
            }
          />
        </label>
      </div>
    </>
  );
}
