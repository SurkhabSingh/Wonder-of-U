# Wonder of U

Turn what you watch into Anki sentence cards.

Wonder of U records the audio playing on your computer, transcribes it locally with
Whisper, and lets you turn individual sentences into Anki cards — each with its own
audio clip cut from the recording. It is built for studying a language from the video
you were going to watch anyway.

Your recordings, transcripts, and clips stay on your machine. Nothing is uploaded
unless you translate a sentence (which uses the optional browser extension) or push a
card to Anki.

## What it does

- **Record** the audio your computer is playing — a video, a podcast, a stream — with
  a global shortcut, from the tray.
- **Import** audio and video files you already have, or paste **YouTube** links and
  queue them up.
- **Transcribe** locally with Whisper. Nothing leaves your machine, and you choose
  when to spend the compute.
- **Read** the transcript beside the audio, with per-sentence playback.
- **Mine** a sentence into an Anki card with its own audio clip, using your existing
  deck, note type, and field mapping.
- **Translate** a transcript into a language you choose — optional, and requires the
  browser extension (see below).

## Install

1. Download the installer (`Wonder of U_<version>_x64-setup.exe`) from the
   [Releases](https://github.com/SurkhabSingh/Wonder-of-U/releases) page.
2. Windows will show **"Windows protected your PC — Unknown publisher."** This app is
   not code-signed, so this warning is expected. Click **More info → Run anyway** if
   you are comfortable with that.
3. Run it. On first launch, open **Setup** and download Whisper and its model.

Windows 10 or 11, 64-bit.

### What to expect on first run

- **A large download.** Whisper's runtime and model plus FFmpeg add up to several
  hundred megabytes, so you need an internet connection. Setup shows progress and you
  can pause, resume, or cancel. This happens once — nothing is bundled in the
  installer, which is why it is small.
- **A Windows Firewall prompt.** The app opens a listener on `127.0.0.1:8791` so the
  browser extension can deliver translations. It is loopback-only and never accepts
  connections from outside your machine. You can decline it if you do not want
  translation.
- On Windows 10, the installer may fetch Microsoft's WebView2 runtime if it is not
  already present. Windows 11 ships with it.

## Optional extras

Everything below is optional. Without them, recording, transcription, the library,
and mining still work.

| For | You need | Without it |
|---|---|---|
| Pushing cards to Anki | **Anki** running, with the **AnkiConnect** add-on (listens on `8765`) | Mining is unavailable; everything else works |
| Translating transcripts | The **Wonder of U browser extension** and its native host | Translation jobs queue and time out; the app is otherwise unaffected |

The browser extension currently has to be loaded unpacked with Chrome's developer
mode, and its native host needs **Node.js** on your PATH. See
`browser-wonder-of-u/README.md`. This is the roughest part of the setup and is not
yet automated.

Furigana generation expects an Anki add-on on port `8766` that is **not yet
published**, so treat that feature as unavailable for now.

## Where your files go

- **Recordings, transcripts, and translations:** `Documents\Wonder of U Recordings`
- **Whisper, FFmpeg, and yt-dlp:** the app's local data directory

Both are configurable in **Settings → Storage**. If you point the recordings folder at
somewhere that already holds audio, the app will adopt those files into its library —
so prefer a folder of its own.

## Third-party tools

Wonder of U downloads these at first run rather than bundling them. They are separate
programs under their own licenses, run as separate processes, and are not
redistributed by this installer.

- [whisper.cpp](https://github.com/ggml-org/whisper.cpp) — transcription
- [FFmpeg](https://github.com/BtbN/FFmpeg-Builds) — audio clipping and conversion
- [yt-dlp](https://github.com/yt-dlp/yt-dlp) — YouTube import

## Development

```powershell
npm install
npm run check                                    # tsc --noEmit
cargo test --manifest-path .\src-tauri\Cargo.toml
npm run tauri dev                                # run it
npm run tauri build                              # build the installer
```

`npm run tauri build` runs `tsc` first, so a TypeScript error fails the whole bundle.
The installer lands in `src-tauri\target\release\bundle\nsis\`.

## License

MIT — see [LICENSE](LICENSE).
