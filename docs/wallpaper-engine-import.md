# Importing settings from Wallpaper Engine

WallForge can read the settings file of the Wallpaper Engine app and carry over what has an equivalent here.

## Where the file is

Wallpaper Engine keeps one `config.json` in its install folder:

| Where you used Wallpaper Engine | Path |
|---|---|
| Windows | `C:\Program Files (x86)\Steam\steamapps\common\wallpaper_engine\config.json` (or the same folder inside another Steam library) |
| Linux through Proton | `~/.local/share/Steam/steamapps/common/wallpaper_engine/config.json` |

The file only exists after Wallpaper Engine has run at least once. Installing it through Steam on Linux without
starting it does not create one.

## How to import

- **In the app:** Settings > Wallpaper Engine > Import settings. If a `config.json` is found in one of your Steam
  libraries there is an **Import** button for it; **Choose file** / **Other file** lets you pick one from anywhere,
  for example a mounted Windows drive.
- **From a terminal:** `./wallforge.sh --import-we [path/to/config.json] [--dry-run]`. Without a path it uses the
  file found in your Steam libraries. `--dry-run` only prints what would be imported.

The app shows a summary first. Nothing is written until you press **Import**. If the file names the wallpaper that
was active and it is installed here, **Import and set wallpaper** also applies it.

## What is imported

| Wallpaper Engine | WallForge |
|---|---|
| A wallpaper's customised properties (`wproperties`, or `wallpaperproperties` in older files) | The same properties in the wallpaper's panel. Supported types: switch, slider, colour, combo, text. |
| A wallpaper's `volume` | Volume in the wallpaper's panel (scaled by the global volume) |
| `fps` | Settings > Playback > Frame rate limit |
| `playbackfullscreen` (run / pause / stop) | Settings > Effects and power > Keep playing under fullscreen apps |
| The selected wallpaper per monitor | Offered as "Import and set wallpaper" |

Wallpapers are matched by their Workshop id, which is part of the path Wallpaper Engine stores, so drive letters and
folder locations do not matter. Wallpapers that are not installed here are left out and counted in the summary;
subscribe to them and import again to pick up their settings.

A value is only imported if the wallpaper still has that property and, for combos, that option. Values equal to the
wallpaper's default are skipped. Your existing WallForge settings for other wallpapers are kept.

### Monitors

Wallpaper Engine names monitors by Windows device paths or `Monitor0`, `Monitor1`, which have no reliable Linux
equivalent. So:

- Properties are per wallpaper here, not per monitor. If a wallpaper had different settings on two monitors, the
  monitor it was active on wins.
- The active wallpaper is only offered when the mapping is unambiguous: the same wallpaper on every monitor, or
  `Monitor0..N` entries that fit your outputs in order.

## What is not imported

| Setting | Why |
|---|---|
| Playlists | WallForge has no playlists yet. The summary tells you how many were found. |
| Alignment, flip, playback speed, per-wallpaper colour adjustments (brightness, contrast, saturation, hue) | No equivalent yet |
| Image, file and folder properties | They hold Windows paths. Set them again in the wallpaper's panel. |
| Quality presets (MSAA, post-processing, resolution), pause rules other than fullscreen, application rules | No equivalent |
| Wallpaper Engine's browser filters, favourites folders and UI preferences | Not relevant here |

## Undoing an import

Imported property values behave like values you set by hand: **Reset settings** in a wallpaper's panel returns that
wallpaper to its defaults (and its volume to 100). The global frame rate and fullscreen rule are ordinary settings.
WallForge's own settings file is `~/.config/wallforge/config.json`; copy it first if you want a full backup.
