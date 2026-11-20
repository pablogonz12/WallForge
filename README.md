<p align="center"><img src="assets/logo-banner.png" alt="WallForge" width="720"></p>

# WallForge

### Your Wallpaper Engine library, running natively on Linux. One small Rust renderer, no Windows, no Wine.

[![CI](https://github.com/pgm1207/WallForge/actions/workflows/ci.yml/badge.svg)](https://github.com/pgm1207/WallForge/actions/workflows/ci.yml)

WallForge is a GTK4 / libadwaita app and a small GPU renderer (`wallforge-render`) that plays the
[Wallpaper Engine](https://store.steampowered.com/app/431960/) wallpapers you already own on any Wayland desktop
with layer-shell support (niri, Hyprland, Sway, ...).

| | |
|---|---|
| **Self-contained** | Its own renderer plays video and scene wallpapers; web wallpapers run in WebKit. No other engine is needed. |
| **Light on the GPU** | VAAPI hardware decode, zero-copy import, compressed textures kept compressed, pauses when hidden. A 1080p video wallpaper takes about 93 MiB of VRAM. |
| **Looks right by default** | Auto fit fills the screen (ultrawide art is cropped) and keeps portrait art whole. |
| **Brings your setup along** | Imports your settings from Wallpaper Engine's own `config.json`. |
| **Easy to share** | One click copies a wallpaper's Steam Workshop link; paste a link to open that wallpaper. |
| **Your palette follows** | Noctalia, matugen, wallust, pywal or your own command pick up each wallpaper's colours. |

## Contents

- [What plays](#what-plays)
- [Features](#features)
- [Install](#install)
- [Share a wallpaper](#share-a-wallpaper)
- [Import your Wallpaper Engine settings](#import-your-wallpaper-engine-settings)
- [Wallpaper Engine's assets](#wallpaper-engines-assets)
- [Command line](#command-line)
- [Updates](#updates)
- [When a wallpaper looks wrong](#when-a-wallpaper-looks-wrong)
- [More documentation](#more-documentation)

## What plays

| Wallpaper type | Status |
|---|---|
| Video | Works: hardware decode, audio, gapless loop, volume and mute. |
| Scene (2D) | Works for most wallpapers: layers, multi-pass effects, particle systems, embedded video layers, text layers, your property edits. See [compatibility](docs/compatibility.md) for what is still missing. |
| Web | Works, in WebKit, with the wallpaper's properties. Needs `webkitgtk-6.0` and `gtk4-layer-shell`. Audio visualisers get silence for now. |
| 3D scenes | Not supported. WallForge tells you before you apply one. |
| Application | Windows programs, not supported on Linux. |

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
- **Rotation**: change the wallpaper on a timer, at random or in library order, optionally only through favourites.
- **Low cost by default**: one small renderer per screen, still scenes drawn once and then frozen, a render
  resolution setting, and battery saving that lowers the frame rate (or pauses everything) when you unplug.
- **Safe**: a crash shows a toast with a "View log" button, a dead renderer is restarted automatically, broken files
  are caught by a watchdog, and unsupported 3D scenes are detected before you apply them.
- **Sensible defaults**: anime and mature content are hidden until you switch them on in Settings.
- Keyboard: `Ctrl+F` search, `Ctrl+R` random, `Ctrl+,` settings, `F5` rescan.

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
cd wallforge
./install.sh        # builds the renderer (about a minute), adds a menu entry and icon
./wallforge.sh
```

### AUR

A `wallforge-git` PKGBUILD is in [`packaging/aur/`](packaging/aur). It is not published on the AUR yet; build it
with `makepkg -si` from that folder.

## Performance and battery

WallForge renders each wallpaper in one small process per screen and keeps still scenes frozen: a scene whose effects do
not depend on time is drawn once and then sleeps until something changes.

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

### Rotation

Settings > Playback > **Change wallpaper every** switches all screens to a new wallpaper every N minutes (0 turns it
off). *Pick a random wallpaper* chooses at random; with it off, the library order is followed. *Rotate only through
favourites* keeps the rotation to wallpapers you starred. Rotation waits while wallpapers are paused by battery saving,
and a wallpaper that is already showing is never picked again immediately.

If a wallpaper stops on its own, WallForge restarts it; after three failures in five minutes it leaves the screen idle
and says so in the playing bar instead of looping on a crash. The log is under Settings > About > View log.

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
crash toast. [docs/troubleshooting.md](docs/troubleshooting.md) covers the common problems.

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
