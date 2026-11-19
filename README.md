<p align="center"><img src="assets/logo-banner.png" alt="Wallforge" width="720"></p>

# Wallforge

A GTK4 / libadwaita app that makes [Wallpaper Engine](https://store.steampowered.com/app/431960/) wallpapers
usable on Linux. It drives [linux-wallpaperengine](https://github.com/Almamu/linux-wallpaperengine) and adds what
that engine lacks: a library, per-wallpaper settings, Workshop browsing, per-monitor control and palette sync.

## Features

- **Library** of everything you subscribed to, with search, type filter, sort, favourites and a random button.
- **All property types** editable per wallpaper (sliders, colours, combos, text, and image slots), applied live.
- **Image slots work.** The engine ignores user-image properties; Wallforge bakes them into a patched copy of the scene.
- **Discover tab** to browse the Steam Workshop, with one-click Get through your running Steam client (experimental: if the click fails, Steam opens the page for you).
- **Per monitor**: apply to one screen or all, ultrawide wallpapers skipped on 16:9 screens by default.
- **Sensible defaults**: crop-to-fill scaling, anime and mature content hidden (switch them on in Settings).
- **Palette sync** for Noctalia, matugen, wallust, pywal, or your own command.
- **Works on** niri, Hyprland, Sway and other wlroots compositors (monitor detection falls back to GDK).
- **Autostart**: `wallforge.sh --restore` re-applies your saved wallpapers at login (toggle in Settings).
- Engine crash toast with a "View log" button, remembered window size, keyboard shortcuts
  (`Ctrl+F` search, `Ctrl+R` random, `Ctrl+,` settings, `F5` rescan).
- Detects unsupported 3D scenes up front instead of failing with an engine error.

## You do not need Wallpaper Engine installed, but you do need to own it

The engine needs Wallpaper Engine's `assets/` folder. Wallforge never ships those files. Use whichever you prefer:

1. **Fetch from Steam (about 85 MB).** Settings > Setup > Fetch opens a terminal where you sign in to Steam with
   [DepotDownloader](https://github.com/SteamRE/DepotDownloader) (pinned release, checksum verified). Your password
   goes to DepotDownloader only, never to Wallforge. An account that does not own Wallpaper Engine is refused by Steam.
2. **Install it in Steam** (about 0.8 GB) and Wallforge uses or copies its `assets/` folder.

Workshop wallpapers themselves are downloaded by the Steam client when you subscribe.

## Requirements

- Python 3.11+, GTK 4, libadwaita 1.5+, PyGObject, Pillow, aiohttp, ffmpeg (for palette stills)
- [linux-wallpaperengine](https://github.com/Almamu/linux-wallpaperengine) installed (AUR: `linux-wallpaperengine-git`)
- Steam running, signed in to an account that owns Wallpaper Engine, for Discover > Get
- A Wayland compositor with layer-shell support (niri, Hyprland, Sway, ...)

## Install and run

**Arch / CachyOS (AUR, once published):** `yay -S wallforge-git`

**From source:**

```bash
git clone https://github.com/pgm1207/wallforge
cd wallforge
./install.sh        # menu entry and icon (optional)
./wallforge.sh
```

## Updates

Wallforge checks GitHub once a day (switch it off in Settings > Updates). A git clone updates itself with an
"Update" button in the notification (`git pull --ff-only`, restart afterwards). The AUR package only tells you
and leaves the update to your package manager. The package lives in `packaging/aur/`.

## Disclaimer

Not affiliated with Wallpaper Engine / Wallpaper Engine Team or Valve. No Wallpaper Engine files are included or
redistributed; they come from your own Steam account. MIT licensed.
