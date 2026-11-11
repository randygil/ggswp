# ggswp

**English** · [Español](README.es.md)

Automatic Fortnite match recorder for Windows. It detects when each match starts and ends,
records the screen (what you see) with audio, marks your eliminations on the timeline,
generates clips and lets you edit them, send them to your phone or upload them to Google Drive.

A single desktop app (Tauri + React), no OBS needed: capture is built in. The interface comes
in English and Spanish.

**Download:** the installer is in [Releases](https://github.com/randygil/ggswp/releases).
Once installed, the app tells you when a new version is available.

## What it does

- **Records on its own** every Fortnite match and saves it to the gallery, with the match
  timeline as a "signature" of events.
- **Eliminations, knocks, landing and victory**, exact, taken from the Fortnite replay.
- **Opened chests**: reads the XP popups ("+600 PE · COFRE") in the video with Windows OCR.
- **Stats**: final placement, damage dealt and taken, accuracy, distance; a history that does not
  shrink when matches expire, and a page with match-by-match charts.
- **Automatic clips** of every elimination, hotkey clips (last 30 s) and a timeline editor.
- **Match recap** in a single video and a **vertical 9:16 version** (TikTok / Shorts / Reels)
  with your elimination counter.
- **Google Drive**: uploads only while you are not playing, with a compressed copy (HEVC with
  NVENC, about 4 times lighter). What is already in Drive is deleted from the PC after 7 days and
  for good after 15 (kept matches never expire). All of it is configurable.
- **Light clips**: saved as H.264 at 1080p (about 1 MB per second) instead of at recording
  quality, ready for WhatsApp and Instagram. The editor exports in horizontal, vertical 9:16 or
  **mobile mode** (9:16 with health and weapons in a strip you can move to any height; the zones
  are adjusted by dragging over the video, with a live preview; you can add loose pieces such as
  the minimap and save the layout as a template) and in three qualities.
- **Audio on separate tracks**: the game is recorded apart from everything else (Discord, music,
  browser) and from the microphone. The full match keeps it all; each clip carries only what
  you pick.
- **Editor effects**: text over the clip, slow motion and zoom on the crosshair for a stretch,
  background music and volume. It also joins several clips into one, in the order you choose.
- **Best play**: when each match ends it picks its best clip and leaves a 9:16 version ready to
  upload. **Weekly recap**: one video with the best of the last 7 days.
- **Other games**: records the whole session of the games you add in Settings (no event
  detection; hotkeys work the same).
- **Sharing**: copy the video to the clipboard, the Windows "Share" panel, save as, to your
  phone over Wi-Fi, or Drive with a QR code. Highlights menu over the game and right-click menus.

## How it works

| Piece | Technique |
|---|---|
| Match detection | Reads `%LOCALAPPDATA%\FortniteGame\Saved\Logs\FortniteGame.log` live: `LoadMap` to a server → loading, `MatchState … InProgress` → start, `EAthenaGamePhase::Aircraft` → battle bus, `Match finished for … end game` → you were eliminated or won, `LoadMap … Frontend` → lobby. |
| Video | Desktop Duplication (`ddagrab`) of the monitor the game is on → the graphics card's encoder: NVENC (NVIDIA), AMF (AMD) or Quick Sync (Intel), in H.264/HEVC/AV1; with none of them, software `libx264`. At startup it tests which ones work. With NVENC and AMF frames never leave the GPU. |
| Audio | WASAPI: loopback of the game's process, loopback of everything else, and an optional microphone, each on its own track (plus one with the mix, which is what any player plays). If Windows cannot split by process, loopback of the output device. Each packet is placed by its QPC timestamp, the same clock as the video, so sync is exact. |
| Eliminations | When the match ends, it reads the `.replay` in `Saved\Demos` (`playerElim` events, AES-256 encrypted with the key in the header): who eliminated whom, knocks, distance and exact time. If there is no replay, it uses the highlight notices Fortnite sends to NVIDIA (`HLSetVideo`). |
| Anti-cheat | Nothing is injected into the game and its memory is never read: only files Fortnite writes to disk (the same thing Medal does). |

## Requirements

- Windows 10/11 with an NVIDIA, AMD or Intel card (without a hardware encoder it records in
  software, which uses a lot of CPU). Splitting the game's audio needs Windows 10 2004 or later.
- In Fortnite: **Settings → Game → Replays enabled** (to detect eliminations) and
  **Window mode: Windowed Fullscreen** if you want to see the notices and the menu over the game.
- For development: Node 20+, stable Rust and ffmpeg with `ddagrab` (6.0 or later).

## Usage

The app lives in the system tray. When you enter a match it starts recording on its own and,
when the match ends, it processes the video: events, thumbnail and automatic clips of every
elimination (consecutive eliminations are merged into one clip: "Double elimination",
"Victory Royale"…).

### Global hotkeys (changeable in Settings)

| Hotkey | Action |
|---|---|
| `Alt+F8` | Save the last 30 s (inside a match or, with the buffer on, at any time while Fortnite is open) |
| `Alt+F7` | Mark the moment on the timeline |
| `Alt+F11` | Start / stop manual recording |
| `Alt+Shift+G` | Highlights menu over the game |

`Alt+F9`/`Alt+F10`/`Alt+Z` are not used by default because NVIDIA uses them.

### Editor

In the match view: `I` / `O` mark the start and end, you drag the edges on the timeline, and
**Export clip** cuts frame-exact (re-encodes with the graphics card). Clicking an event or a clip
on the timeline jumps to that moment and marks it, ready to trim and export. **Effects** opens
the text, slow motion with zoom, music and audio sources of the clip. `←`/`→` 5 s,
`Shift+←/→` 1 s, `,`/`.` frame by frame, `Space` pauses, `F` fullscreen.

### Sharing

Before sharing you pick the format: **for sharing** (1080p, ~1 MB/s), **light** (720p, under
16 MB) or **original**. If needed, the copy is encoded on the
spot and kept in the match's `share/` folder.

- **File**: copy the video (to paste it with Ctrl+V into WhatsApp, Discord or
  Telegram), open the Windows "Share" panel, save as, or show it in its folder.
- **Wi-Fi**: a QR code with a local link (`http://<your-ip>:8765/…`); the phone opens the video
  and can download it. It must be on the same network. The link expires after 24 h. The address
  of the Ethernet or Wi-Fi adapter is used (not a VPN's) and, if the Windows firewall blocks the
  app, the dialog offers to allow it (it asks for administrator permission).
- **Google Drive**: uploads the video to the `ggswp` folder in your Drive and generates a QR code
  to the link (visible to anyone who has the link).

### Setting up Google Drive

1. In [Google Cloud Console](https://console.cloud.google.com/) create a project and enable **Google Drive API**.
2. **OAuth consent screen**: type *External*. Add your account as a test user or, better,
   publish it ("In production"): the `drive.file` scope needs no verification, and that way
   access does not expire after 7 days.
3. **Credentials → Create OAuth client ID → Desktop app**. Copy the *Client ID* and the
   *Client secret* into ggswp → Settings → Google Drive → Connect.

ggswp can only see the files it uploads itself. Optionally it uploads each match and/or its
clips automatically when the match ends.

## Where everything is stored

- Recordings: `%USERPROFILE%\Videos\ggswp\<date>\` (`match.mp4`, `meta.json`, `thumb.jpg`, `clips\`).
- Settings: `%APPDATA%\com.ggswp.app\settings.json` (and the history in `stats.json`).
- Temporary buffer: `%LOCALAPPDATA%\com.ggswp.app\buffer\` (deleted automatically).

## Development

```bash
npm install
npx tauri dev        # app in development mode
npx tauri build      # NSIS installer in src-tauri/target/release/bundle/nsis/
```

For the installer, copy an `ffmpeg.exe` with `ddagrab` into `src-tauri/bin/` (it is bundled as a
resource; if it is missing, the app uses the `ffmpeg` on the PATH).

Tests (the ones marked `ignore` use the real Fortnite files and record the screen):

```bash
cd src-tauri
cargo test                                  # log parser
cargo test -- --ignored --nocapture         # real replay, 6 s capture
cargo test --test pipeline -- --ignored     # recording + analysis + clips end to end
```

### Code map

```
src-tauri/src/
  fortnite/log.rs     log tailing and match events
  fortnite/replay.rs  .replay reader (eliminations)
  capture/            ddagrab+NVENC, WASAPI, final mux, Win32 utilities
  engine.rs           state machine, replay buffer, post-processing
  library.rs          on-disk library, timeline, clips
  overlay.rs          notices and highlights menu over the game
  hotkeys.rs          global hotkeys
  share.rs            Wi-Fi server + QR, local network and firewall
  winshare.rs         clipboard and Windows "Share" panel
  export.rs           output formats, mobile mode and editor effects (text, slow motion, music)
  gdrive.rs uploads.rs  Google Drive
  fortnite/chests.rs  opened chests (OCR of the XP popups)
  montage.rs          match recap, weekly recap, clip joining and vertical version
  retention.rs        expiry (PC and Drive)
  stats.rs            stats history
src/                  React interface (gallery, clips, editor, stats, settings, overlay)
scripts/release.ps1   publishes a version to GitHub Releases (signed for the updater)
```

### Publishing a version

1. Bump the version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json`, and commit.
2. `powershell -File scripts/release.ps1 -Notes "What changed"`

The script builds, signs the installer with the key in `%USERPROFILE%\.tauri\ggswp.key` (it never
goes into the repository), generates `latest.json` and creates the release. Installed copies
download it on their own.

## Third-party licenses

The installer includes `ffmpeg.exe` (FFmpeg, GPLv3), which ggswp runs as a separate program. See
[`src-tauri/bin/FFMPEG-NOTICE.txt`](src-tauri/bin/FFMPEG-NOTICE.txt) for the version and source code.

## Known limitations

- With Fortnite in exclusive *Fullscreen* the notices and the menu are not visible (recording
  works the same).
- The AMD (AMF) and Intel (Quick Sync) encoders are implemented, but only NVENC and the software
  encoder have been tested: if they fail on your card, pick "Software" in Settings.
- The Fortnite replay does not say when you revive someone or how many opponents were left: the
  app recognises long shots, streaks, the winning elimination and the ones you get after being
  knocked, but not revives or 1-versus-many situations.
- In other games there are no events or automatic clips: only the recording and the hotkeys.
- HEVC/AV1 videos play inside the app only if Windows has the codecs; H.264 always works.
