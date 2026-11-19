<p align="center"><img src="assets/logo-banner.png" alt="Wallforge" width="720"></p>

# Wallforge

### Your Wallpaper Engine library, running natively on Linux. One small Rust renderer, no Windows, no Wine, no extra engine.

Wallforge is a GTK4 / libadwaita app and a tiny GPU renderer (`wallforge-render`) that plays the
[Wallpaper Engine](https://store.steampowered.com/app/431960/) wallpapers you already own on any Wayland desktop.

| | |
|---|---|
| **Self-contained** | Its own Rust renderer plays video and scene wallpapers. `linux-wallpaperengine` is not needed. |
| **Light on the GPU** | VAAPI hardware decode, zero-copy import, compressed textures kept compressed, pauses when hidden. A 1080p video wallpaper takes about 93 MiB of VRAM, 53 MiB with software decode. |
| **Looks right by default** | **Auto fit** picks fill or fit per wallpaper and per screen, so ultrawide, portrait and 16:9 art all land well without touching a setting. |
| **Your palette follows** | Noctalia, matugen, wallust, pywal or your own command pick up each wallpaper's colours. |
| **Stays up to date** | A git install updates itself in the background, checks that the new version loads and rolls back if it does not. |

## Features

- **Library** of everything you subscribed to, with search, type filter, sort, favourites and a random button.
- **All property types** editable per wallpaper (sliders, colours, combos, text, and image slots), applied live.
- **Discover tab** to browse the Steam Workshop, with one-click Get through your running Steam client (experimental: if the click fails, Steam opens the page for you).
- **Per monitor**: apply to one screen or all, ultrawide wallpapers skipped on 16:9 screens by default.
- **Auto scaling** (default): crops when little is lost, shows the whole picture when the aspect ratios differ a lot. Fill, fit and stretch stay one click away in Settings.
- **Sensible defaults**: anime and mature content hidden (switch them on in Settings).
- **Works on** niri, Hyprland, Sway and other wlroots compositors (monitor detection falls back to GDK).
- **Autostart**: `wallforge.sh --restore` re-applies your saved wallpapers at login (toggle in Settings).
- **Safe**: crash toast with a "View log" button, a watchdog for broken files, and unsupported 3D scenes are detected up front.
- Keyboard shortcuts: `Ctrl+F` search, `Ctrl+R` random, `Ctrl+,` settings, `F5` rescan.

## What plays today

| Wallpaper type | Status |
|---|---|
| Video | Works: hardware decode, audio, gapless loop, volume and mute. |
| Scene (2D) | Native renderer, in active development. Layers, effects and your property edits work; particles and audio-reactive effects are next. If you also have `linux-wallpaperengine` installed it is used as a fallback. |
| Web | Not supported yet. |
| Application | Windows programs, never supported on Linux. |
| 3D scenes | Not supported. Wallforge tells you before you apply one. |

Details and the renderer's flags are in [`render/README.md`](render/README.md).

## You do not need Wallpaper Engine installed, but you do need to own it

Scene wallpapers need Wallpaper Engine's `assets/` folder (shaders and textures). Wallforge never ships those files.
Video wallpapers do not need them at all. Use whichever you prefer:

1. **Fetch from Steam (about 85 MB).** Settings > Setup > Fetch opens a terminal where you sign in to Steam with
   [DepotDownloader](https://github.com/SteamRE/DepotDownloader) (pinned release, checksum verified). Your password
   goes to DepotDownloader only, never to Wallforge. An account that does not own Wallpaper Engine is refused by Steam.
2. **Install it in Steam** (about 0.8 GB) and Wallforge uses or copies its `assets/` folder.

Workshop wallpapers themselves are downloaded by the Steam client when you subscribe.

## Requirements

- Python 3.11+, GTK 4, libadwaita 1.5+, PyGObject, Pillow, aiohttp, ffmpeg
- Rust/cargo to build the renderer (`install.sh` and the first start do it), plus ffmpeg, libva and EGL
- Steam running, signed in to an account that owns Wallpaper Engine, for Discover > Get
- A Wayland compositor with layer-shell support (niri, Hyprland, Sway, ...)

## Install and run

**Arch / CachyOS (AUR, once published):** `yay -S wallforge-git`

**From source:**

```bash
git clone https://github.com/pgm1207/wallforge
cd wallforge
./install.sh        # builds the renderer, adds a menu entry and icon
./wallforge.sh
```

## Updates

Wallforge checks GitHub at start and every few hours (switch it off in Settings > Updates). A git clone updates
itself like Steam does: it downloads the new version in the background (`git pull --ff-only`), checks that it loads
(and rolls back if not), then shows "Restart now". Turn that off to get an "Update" button instead. Local edits in
the folder are never overwritten. The AUR package only tells you and leaves the update to your package manager. The package lives in `packaging/aur/`.

## Contributing

Bug reports with a wallpaper id and the log (`~/.local/state/wallforge/<output>.log`) help the most, especially for
scene wallpapers that render wrongly. Pull requests welcome.

## Disclaimer

Not affiliated with Wallpaper Engine / Wallpaper Engine Team or Valve. No Wallpaper Engine files are included or
redistributed; they come from your own Steam account. MIT licensed.
