<p align="center"><img src="assets/logo-banner.png" alt="WallForge" width="720"></p>

<h3 align="center">Your Wallpaper Engine library, running natively on Wayland.<br>One small Rust renderer. No Wine, no second engine.</h3>

<p align="center">
  <a href="https://github.com/pgm1207/WallForge/actions/workflows/ci.yml"><img src="https://github.com/pgm1207/WallForge/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/pgm1207/WallForge/releases/latest"><img src="https://img.shields.io/github/v/release/pgm1207/WallForge?color=blue" alt="Latest release"></a>
  <img src="https://img.shields.io/badge/license-MIT-green" alt="MIT license">
  <img src="https://img.shields.io/badge/Wayland-layer--shell-6e56cf" alt="Wayland layer-shell">
</p>

<p align="center"><img src="assets/demo.gif" alt="A scene wallpaper with a fluid-simulation effect, rendered live by WallForge" width="760"></p>

<p align="center"><sub>Recorded from the renderer: a Wallpaper Engine scene with a multi-pass fluid simulation, no post-processing.</sub></p>

**WallForge** is a GTK4 / libadwaita app *and* the renderer behind it. It plays the
[Wallpaper Engine](https://store.steampowered.com/app/431960/) wallpapers you already own on any Wayland desktop with
layer-shell support — niri, Hyprland, Sway, river — and it manages them: search, per-wallpaper settings, audio,
rotation, palette sync.

```bash
# Arch / CachyOS
sudo pacman -S python-gobject gtk4 libadwaita python-pillow python-aiohttp ffmpeg libva rust clang \
               webkitgtk-6.0 gtk4-layer-shell
git clone https://github.com/pgm1207/WallForge && cd WallForge && ./install.sh && ./wallforge.sh
```

You need to own Wallpaper Engine on Steam (for the Workshop library and its shared assets); WallForge never ships or
redistributes its files.

## Why WallForge

| | |
|---|---|
| **One app, not a pile** | Library, settings, Discover, share links and the renderer in a single GTK4 window. Nothing else to install or configure. |
| **Light by design** | One small Rust process per screen. Still scenes are drawn once and then sleep. A 4K video wallpaper costs about 2% of one CPU core. |
| **Looks right by default** | Auto fit fills the screen (ultrawide art is cropped) and keeps portrait art whole. No stretched wallpapers. |
| **Brings your setup along** | Imports your Wallpaper Engine `config.json`: per-wallpaper properties, volumes, frame rate limit, fullscreen rule. |
| **Plays the hard ones** | Multi-pass effects, layer render targets, particles, embedded video, text with glow/outline, audio-reactive visualisers. |
| **Fits your desktop** | Palette sync with Noctalia, matugen, wallust or pywal; per-monitor wallpapers; rotation; restore at login; battery saving. |

## What it plays

Every scene wallpaper in our test library (77 installed Workshop scenes) starts with nothing skipped, except one 3D
scene, which WallForge detects and refuses instead of showing a black screen. The full matrix, including what is still
approximate, is in [docs/compatibility.md](docs/compatibility.md).

| Wallpaper type | Status |
|---|---|
| Video | Hardware decode (VAAPI, zero-copy DMA-BUF), audio, gapless loop, volume and mute. |
| Scene (2D) | Layers, multi-pass effects, render-target sampling, particles, embedded video, text layers, property edits. SceneScript is not executed. |
| Web | WebKit on the wallpaper layer, with the wallpaper's properties. |
| 3D scenes | Not supported; detected before you apply one. |

## Screenshots

<img src="assets/screenshot-library.jpg" alt="The WallForge library with cover art for every subscribed wallpaper" width="100%">

<img src="assets/screenshot-settings.jpg" alt="Settings: playback, rotation, render resolution, battery saving, effects" width="100%">

## Performance

Measured on a Ryzen 9 9900X with a Radeon RX 9070 (RADV), 3840×2160 output, 30 fps, one wallpaper:

| Wallpaper | CPU (one core) | GPU | Renderer memory |
|---|---|---|---|
| 4K video, hardware decoded | ~2% | ~6% (idle desktop is ~5%) | 137 MB |
| Static scene | ~1% | idle: drawn once, then sleeps | 108 MB |
| Fluid-simulation scene (19 effect passes) | ~4% | ~20% | 297 MB |

Settings that trade a little sharpness for a lot less GPU time:

- **Render resolution (%)**: scene wallpapers are drawn at 25–100% of the screen size and upscaled with linear
  filtering. Videos and web wallpapers always play at native resolution. 100% is the default; 75% is a good
  compromise on a laptop or a shared GPU, 50% makes a heavy scene comfortable on an integrated GPU.
- **Frame rate limit**: 30 fps by default (5–240). 24 is enough for most wallpapers and nearly halves the work of 60.
- **Disable particles**, **Disable parallax**, **Disable mouse interaction**: turn off the effects that cost the most
  and that you are least likely to notice under your windows.
- **Reduce frame rate and resolution on battery**: while discharging, the limit drops to 30 fps and the render scale to
  75% at most, without touching your settings.
- **Pause wallpapers on battery**: stops every wallpaper while discharging and brings them back when you plug in.

If a wallpaper stops on its own, WallForge restarts it; after three failures in five minutes it leaves the screen idle
and says so in the playing bar instead of looping on a crash. The log is under Settings > About > View log.

## Rotation

Settings > Playback > **Change wallpaper every** switches to a new wallpaper every N minutes (0 turns it off).
*Pick a random wallpaper* chooses at random; with it off, the library order is followed. *Rotate only through
favourites* keeps the rotation to wallpapers you starred. *Give each screen its own wallpaper* rotates each monitor
independently, so two screens get two different wallpapers; with it off they all change to the same one. Rotation
waits while wallpapers are paused by battery saving, never picks a wallpaper that is already showing, and leaves
screens you stopped by hand stopped. **Ctrl+Shift+N** switches now without waiting for the timer.

While rotation is on, closing the window keeps WallForge running in the background so the timer keeps working; open it
again from the menu (or your launcher) and it comes straight back. **Ctrl+Q** quits for good.

## Features

- **Library** of everything you subscribed to, with search, type filter, sort, favourites and a random button.
- **Every property type** is editable per wallpaper (sliders, colours, combos, text, image slots) and applies live.
- **Volume per wallpaper**, on top of the global volume and mute.
- **Discover tab** browses the Steam Workshop, with one-click Get through your running Steam client
  (experimental: if the click fails, Steam opens the page for you).
- **Share and open links**: copy any wallpaper's Workshop link, or paste one into Discover.
- **Import from Wallpaper Engine**: your customised wallpaper settings, volumes and playback rules.
- **Per monitor**: apply to one screen or all. Ultrawide wallpapers are cropped to fill 16:9 screens, or skipped if you prefer.
- **Auto scaling** by default; fill, fit and stretch are in Settings.
- **Palette sync** with Noctalia, matugen, wallust, pywal or a custom command.
- **Restore at login**: `wallforge.sh --restore` re-applies your saved wallpapers (toggle in Settings).
- **Audio-reactive wallpapers** hear what you play: visualisers and spectrum effects are driven by the system's audio
  output, with no microphone involved, and stay idle when no wallpaper asks for them.
- **Safe**: a crash shows a toast with a "View log" button, a dead renderer is restarted automatically, broken files
  are caught by a watchdog, and unsupported 3D scenes are detected before you apply them.
- **Sensible defaults**: anime and mature content are hidden until you switch them on in Settings.
- Keyboard: `Ctrl+F` search, `Ctrl+R` random, `Ctrl+,` settings, `F5` rescan.

## Contents

- [What plays](#what-plays)
- [Screenshots](#screenshots)
- [Performance](#performance)
- [Rotation](#rotation)
- [Features](#features)
- [Install](#install)
- [Share a wallpaper](#share-a-wallpaper)
- [Import your Wallpaper Engine settings](#import-your-wallpaper-engine-settings)
- [Wallpaper Engine's assets](#wallpaper-engines-assets)
- [Command line](#command-line)
- [Updates](#updates)
- [When a wallpaper looks wrong](#when-a-wallpaper-looks-wrong)
- [More documentation](#more-documentation)

## Install

### Requirements

- A Wayland compositor with layer-shell support (niri, Hyprland, Sway, ...). GNOME Shell does not have it.
- Python 3.11+, GTK 4, libadwaita 1.5+, PyGObject, Pillow, aiohttp, ffmpeg
- Rust (cargo) and clang to build the renderer, plus libva and EGL
- For web wallpapers: `webkitgtk-6.0` and `gtk4-layer-shell`
- Steam, signed in to an account that owns Wallpaper Engine, for downloading Workshop wallpapers

Arch / CachyOS:

```bash
sudo pacman -S python-gobject gtk4 libadwaita python-pillow python-aiohttp ffmpeg libva rust clang \
               webkitgtk-6.0 gtk4-layer-shell
```

### From source

```bash
git clone https://github.com/pgm1207/WallForge
cd WallForge
./install.sh        # builds the renderer (about a minute), adds a menu entry and icon
./wallforge.sh
```

### AUR

A `wallforge-git` PKGBUILD is in [`packaging/aur/`](packaging/aur). It is not published on the AUR yet; build it
with `makepkg -si` from that folder.

## Share a wallpaper

Select a wallpaper and press the **share button** in its panel (next to the favourite star). The link to its Steam
Workshop page is copied to the clipboard, and the toast offers to open the page. Discover cards have the same button.

To open a link someone sent you, paste it into the **Discover** search box and press Enter. If you already have the
wallpaper it opens in your library; otherwise its card appears with a Get button. A bare Workshop id works too.

Only wallpapers that are on the Workshop have a link. The button is greyed out for local projects.

## Import your Wallpaper Engine settings

**Settings > Wallpaper Engine > Import settings** reads the `config.json` that Wallpaper Engine keeps in its install
folder (`steamapps/common/wallpaper_engine/config.json`). If you used Wallpaper Engine on Windows, pick the file from
your Windows drive with **Choose file**. You see what would change before anything is written.

| Imported | Not imported |
|---|---|
| Each wallpaper's customised properties (colours, sliders, switches, combos, text) | Playlists |
| Each wallpaper's volume | Per-wallpaper alignment, playback speed and colour adjustments |
| Frame rate limit | Image and file properties (they point at Windows paths) |
| The fullscreen pause rule | Settings of wallpapers that are not installed here |
| Optionally, the wallpaper that was active | |

From a terminal: `./wallforge.sh --import-we [path/to/config.json] [--dry-run]`.
Details are in [docs/wallpaper-engine-import.md](docs/wallpaper-engine-import.md).

## Wallpaper Engine's assets

You do not need Wallpaper Engine installed, but you do need to own it. Scene wallpapers need its `assets/` folder
(shaders and textures), which WallForge never ships. Video and web wallpapers do not need it. Use either:

1. **Fetch from Steam (about 85 MB).** Settings > Setup > Fetch opens a terminal where you sign in to Steam with
   [DepotDownloader](https://github.com/SteamRE/DepotDownloader) (pinned release, checksum verified). Your password
   goes to DepotDownloader only, never to WallForge. An account that does not own Wallpaper Engine is refused by Steam.
2. **Install it in Steam** (about 0.8 GB). WallForge uses its `assets/` folder directly.

Workshop wallpapers themselves are downloaded by the Steam client when you subscribe.

## Command line

```bash
./wallforge.sh                         # the app
./wallforge.sh --restore               # re-apply the saved wallpapers and exit (for autostart)
./wallforge.sh --import-we [FILE] [--dry-run]   # import Wallpaper Engine settings
```

The renderer and the web player can be run on their own; see [`render/README.md`](render/README.md).

## Updates

WallForge checks GitHub at start and every few hours (switch it off in Settings > Updates). A git clone updates
itself: it pulls the new version in the background (`git pull --ff-only`), checks that it loads, rolls back if it
does not, then offers "Restart now". Local edits in the folder are never overwritten. A package install only tells
you and leaves the update to your package manager.

## When a wallpaper looks wrong

Scene wallpapers are the hard part, and some still render differently from Wallpaper Engine. Please
[open an issue](https://github.com/pgm1207/WallForge/issues/new/choose) with the wallpaper's Workshop link (the share
button copies it) and the log, which is at `$XDG_RUNTIME_DIR/wallforge/<output>.log` or behind "View log" in the
crash toast. Settings > Updates > **Copy diagnostics** puts the version, GPU, renderer report and log lines on the
clipboard in one click. [docs/troubleshooting.md](docs/troubleshooting.md) covers the common problems.

## More documentation

- [docs/compatibility.md](docs/compatibility.md): which Wallpaper Engine features are supported
- [docs/wallpaper-engine-import.md](docs/wallpaper-engine-import.md): exactly what the import reads
- [docs/troubleshooting.md](docs/troubleshooting.md): setup problems, logs, known limits
- [render/README.md](render/README.md): the renderer, its flags and internals
- [CONTRIBUTING.md](CONTRIBUTING.md): building, testing against your library, sending fixes
- [CHANGELOG.md](CHANGELOG.md)

## Disclaimer

Not affiliated with Wallpaper Engine, the Wallpaper Engine Team or Valve. No Wallpaper Engine files are included or
redistributed; they come from your own Steam account. MIT licensed.
