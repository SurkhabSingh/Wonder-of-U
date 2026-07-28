import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { errorMessage } from "../../lib/errors";
import { fileNameFromPath } from "../../lib/format";
import type { JimakuEntry, JimakuFile } from "../../types";

// Finding Japanese subtitles for the video you are about to watch.
//
// Two steps, because Jimaku's API is two steps: search a title, then list its files.
//
// There is deliberately no episode box. Jimaku derives the episode from free-form filenames
// and misses whenever a title is numbered per-season against an absolute number, so the
// filter needed a retry-unfiltered fallback to be usable at all. Every file is listed and
// the filename picks one — which is what a user reads regardless.

/// The name most people will recognise, falling back through what Jimaku actually returns.
function entryLabel(entry: JimakuEntry): string {
  return entry.englishName || entry.name || `Entry ${entry.id}`;
}

export function JimakuSearchPanel({
  hasApiKey,
  videoPath,
  onDownloaded,
  onOpenSettings,
}: {
  hasApiKey: boolean;
  /// Where the file lands — beside the video, so it sits with what it belongs to.
  videoPath: string | null;
  onDownloaded: (subtitlePath: string) => void;
  onOpenSettings: () => void;
}) {
  const [query, setQuery] = useState("");
  const [entries, setEntries] = useState<JimakuEntry[] | null>(null);
  const [entry, setEntry] = useState<JimakuEntry | null>(null);
  const [files, setFiles] = useState<JimakuFile[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function search() {
    setBusy(true);
    setError(null);
    setEntry(null);
    setFiles(null);
    try {
      setEntries(await invoke<JimakuEntry[]>("jimaku_search", { query }));
    } catch (caught) {
      setEntries(null);
      setError(errorMessage(caught, "That search could not be run."));
    } finally {
      setBusy(false);
    }
  }

  async function openEntry(next: JimakuEntry) {
    setBusy(true);
    setError(null);
    setEntry(next);
    try {
      setFiles(await invoke<JimakuFile[]>("jimaku_files", { entryId: next.id }));
    } catch (caught) {
      setFiles(null);
      setError(errorMessage(caught, "Those files could not be listed."));
    } finally {
      setBusy(false);
    }
  }

  async function download(file: JimakuFile) {
    setBusy(true);
    setError(null);
    try {
      const path = await invoke<string>("jimaku_download", {
        url: file.url,
        fileName: file.name,
        videoPath,
      });
      onDownloaded(path);
    } catch (caught) {
      setError(errorMessage(caught, "That file could not be downloaded."));
    } finally {
      setBusy(false);
    }
  }

  if (!hasApiKey) {
    return (
      <div className="info-note">
        <p className="microcopy">
          Searching <strong>Jimaku</strong> for Japanese subtitles needs an API key from{" "}
          <strong>jimaku.cc/account</strong>.{" "}
          <button type="button" className="link-button" onClick={onOpenSettings}>
            Add it in Settings
          </button>
          .
        </p>
      </div>
    );
  }

  return (
    <div className="jimaku-panel">
      <div className="jimaku-search-row">
        <input
          type="text"
          value={query}
          placeholder="Search Jimaku for a title…"
          aria-label="Jimaku title search"
          onChange={(event) => setQuery(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              void search();
            }
          }}
        />
        <button
          type="button"
          className="secondary"
          onClick={() => void search()}
          disabled={busy || !query.trim()}
        >
          {busy ? "Working…" : "Search"}
        </button>
      </div>

      {error ? (
        <div className="update-card error">
          <strong>{error}</strong>
        </div>
      ) : null}

      {entry && files ? (
        <div className="jimaku-results">
          <div className="jimaku-results-header">
            <button
              type="button"
              className="link-button"
              onClick={() => {
                setEntry(null);
                setFiles(null);
              }}
            >
              ‹ Back to results
            </button>
            <span className="microcopy">{entryLabel(entry)}</span>
          </div>
          {files.length === 0 ? (
            <p className="microcopy">
              No subtitle files for this title. Archives are hidden, because the app cannot
              unpack one.
            </p>
          ) : (
            files.map((file) => (
              <button
                key={file.url}
                type="button"
                className="jimaku-result"
                disabled={busy}
                onClick={() => void download(file)}
              >
                {file.name}
              </button>
            ))
          )}
        </div>
      ) : entries ? (
        <div className="jimaku-results">
          {entries.length === 0 ? (
            <p className="microcopy">No matches on Jimaku for that title.</p>
          ) : (
            entries.slice(0, 25).map((item) => (
              <button
                key={item.id}
                type="button"
                className="jimaku-result"
                disabled={busy}
                onClick={() => void openEntry(item)}
              >
                <span>{entryLabel(item)}</span>
                {item.japaneseName ? (
                  <span className="microcopy">{item.japaneseName}</span>
                ) : null}
              </button>
            ))
          )}
        </div>
      ) : null}

      <p className="microcopy">
        {videoPath
          ? `Downloads save beside ${fileNameFromPath(videoPath)}.`
          : "Pick a video first and the subtitle will be saved beside it."}
      </p>
    </div>
  );
}
