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
import { SettingsDisclosure } from "./SettingsDisclosure";

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
      <SettingsDisclosure
        title="Dictionary &amp; Reading"
        defaultOpen={false}
      >
        <div className="info-note">
          <p className="microcopy">
            Hold a key and hover a word to see its dictionary entry &mdash; in the subtitle
            list, and over the video when the overlay is on.{" "}
            <strong>Anki needs to be open.</strong>
          </p>
        </div>

      <div className="settings-grid">
        <label className="field">
          <span className="field-label-with-help">
            <span>Scan with</span>
            <TooltipBadge
              label="?"
              description="The key you hold while hovering a word. Over the video it also decides who gets the mouse: hold it to scan, release it to go back to the player's controls."
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
          <span className="field-label-with-help">
            <span>When you let go</span>
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
          <span className="field-label-with-help">
            <span>Lookup delay</span>
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
          <span className="field-label-with-help">
            <span>Popup text size</span>
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
          <span className="field-label-with-help">
            <span>Subtitles over the video</span>
            <TooltipBadge
              label="?"
              description="Text size for the subtitle line drawn over the video. Only used while that is switched on from the Watch page."
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
          <span className="field-label-with-help">
            <span>Reading font</span>
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

      </SettingsDisclosure>

      <SettingsDisclosure
        title="Jimaku"
        defaultOpen={false}
      >
        <div className="info-note">
          <p className="microcopy">
            Search Jimaku for Japanese subtitles from the Watch page and save them beside
            your video. Get a key from <strong>jimaku.cc/account</strong>.
          </p>
        </div>

      <div className="settings-grid">
        <label className="field field-wide">
          <span className="field-label-with-help">
            <span>API key</span>
            <TooltipBadge
              label="?"
              description="Stored in this app's settings file in plain text, like every other key here. Leave it empty to hide the Jimaku search entirely."
            />
          </span>
          <input
            type="password"
            value={settingsDraft.jimakuApiKey}
            placeholder="Paste your Jimaku API key"
            autoComplete="off"
            spellCheck={false}
            onChange={(event) =>
              onUpdateSettings({ jimakuApiKey: event.currentTarget.value })
            }
          />
        </label>
      </div>
      </SettingsDisclosure>
    </>
  );
}
