# Changelog

## Unreleased

- **niri**: Settings > Desktop > "Fill the niri overview" shows the live wallpaper behind niri's overview (one wallpaper behind
  the workspace cards) by managing `~/.config/niri/wallforge.kdl`; on by default.

## 0.8.0 (2026-11-20)

### New

- **GNOME 45+ support** through a bundled GNOME Shell extension: the renderer runs as a hidden window (`--window`,
  chosen automatically on GNOME) and the extension shows it behind your windows, in the overview and in workspace
  thumbnails. Settings > Setup has an "Install GNOME extension" button; `install.sh` and the packages ship it too.
  Tested on GNOME Shell 51 (video wallpapers); web wallpapers are not supported there yet.
- Monitors are discovered through Mutter on GNOME, so the connector names match.
- KDE Plasma is documented as supported (tested on KWin 6.8). Correction: KWin has implemented `wlr-layer-shell`
  since Plasma 5.20, not 6.7 as 0.7.0's notes said.
- **Add your own wallpapers**: the + button (or `wallforge --import FILE...`) imports video files, images, a web
  wallpaper folder or a **Lively Wallpaper** `.zip` package into a local library next to your Workshop items. Videos
  are linked, not copied (`--copy` copies). Lively zips are unpacked safely (no paths outside the folder, size cap).
- **Discover sources**: a selector for Wallpaper Engine Workshop or **Wikimedia Commons** (free videos, public
  domain / CC0 / CC BY / CC BY-SA only, with licence and author on the card and detail page, and an attribution line
  saved in the project). `wallforge --commons` searches from the terminal.
- **Source badges** on every Discover and Library card (Wallpaper Engine, Lively, Local, Wikimedia), and a source
  filter in the Library.
- **Uninstall** for any installed wallpaper, with a confirmation, stopping it first if it is playing. Workshop items
  open their Steam page to unsubscribe, since deleting the folder would just be re-downloaded.

### Fixed

- Some scenes were drawn upside down (e.g. "Giratina's Entrance at Spear Pillar"): a layer that other objects
  sample was drawn from its bottom-up render-target copy.
- niri's blur/xray showed the preloaded next wallpaper instead of the visible one. The preloaded renderer now starts
  `--hidden` (it loads and renders but presents nothing until it is switched to, still a ~28 ms switch), and the
  wallpaper now sits on the `background` layer by default, which is the layer niri's blur samples.
- Window mode now renders at the monitor's size instead of the 1920x1048 work area, so the wallpaper is no longer
  slightly stretched. Preloading is off in window mode.

## 0.7.0

### New

- **Install with pacman**: every release is built in an Arch container and published to a pacman repository on
  GitHub Pages, so `sudo pacman -Sy wallforge` installs and updates it. The AUR recipe is ready for when AUR
  registration reopens.
- **Check my setup** (Settings > Updates) and `wallforge --doctor`: one read-only check of Wayland, GTK,
  layer-shell, the renderer, Wallpaper Engine assets, the Workshop library, ffmpeg/VAAPI, WebKit, the config and the
  last run's log, with what to fix. Works without a display; exits 1 on failures.
- **Copy diagnostics** (Settings > Updates): version, GPU, the selected wallpaper's renderer report and its log, ready
  to paste into an issue.
- `tools/bench.sh <workshop-id>` reproduces the README's performance table on your machine, against an idle baseline.

### Changed

- **Preloading**: after a switch, the next wallpaper (the one rotation or Ctrl+Shift+N would pick) is loaded on the
  layer below the current one, so the switch to it is a single frame: measured 27–29 ms on the screen, against
  130–180 ms for a cold scene and ~500 ms for a scene with 352 MB of textures or a video. Costs the preloaded
  wallpaper's memory until it is used; toggle in Settings > Effects and power, and it is off in `background` layer mode
  where there is no layer below to hide in.
- **Instant wallpaper switching**: the new renderer is started before the old one is stopped and the old wallpaper
  stays on screen until the new one presents its first frame (110–160 ms for most scenes), so switching is a cut
  instead of a black flash. The renderer gained `--ready-file` for this; web wallpapers still replace each other
  directly.
- **KDE Plasma 6.7+** implements `wlr-layer-shell`, so WallForge draws there; the docs, the FAQ, `--doctor` and the
  renderer's error messages now say so. A compositor without layer-shell (GNOME Shell) gets an explanation and a
  pointer to the tracking issue instead of a protocol bind error.
- Settings > Setup describes the app as it is: WallForge's own renderer plays video, scene and web wallpapers. The
  linux-wallpaperengine row only appears when it is installed, as an optional fallback switch. Status icons render
  on every icon theme, and paths are shown relative to your home folder.

### Documentation

- README rebuilt: one-command install, demo animation, measured performance, FAQ, supported desktops, keyboard
  shortcuts, roadmap. Two stale claims fixed.
- New `docs/comparison.md`, `SECURITY.md` and `CODE_OF_CONDUCT.md`; issue forms ask for `--doctor` and Copy
  diagnostics output.
- `tools/ui_screenshots.sh` regenerates the README screenshots off-screen (GTK Broadway), without touching your
  desktop or your wallpapers.

## 0.6.0

### New

- **Wallpaper rotation**: Settings > Playback changes the wallpaper every N minutes, at random or in library order,
  optionally only through favourites and optionally with a different wallpaper on each screen. `Ctrl+Shift+N` rotates
  immediately.

### Performance and battery

- **Render resolution** (Settings > Effects and power): scene wallpapers can be drawn at 25–100% of the screen size
  and upscaled with one linear blit. Videos and web wallpapers are unaffected.
- **Battery saving**: while discharging, the frame rate is capped at 30 fps and the render scale at 75% (on by
  default); optionally pause every wallpaper on battery and resume when you plug in.
- Web wallpapers follow the reduced frame rate too.

### Compatibility

- **Audio-reactive wallpapers**: the system's audio output is captured (PipeWire/PulseAudio monitor, no microphone)
  and fed to the `g_AudioSpectrum` uniforms, so visualisers and spectrum effects move with your music. Capture starts
  only when a scene's shaders ask for it and can be turned off in Settings.
- **Text layers**: effect chains (glow, outline, shadow) now run on text, point sizes match the wallpapers' previews, date/time `format` patterns are expanded (strftime,
  `{year}`-style and Cafe-style `dd MMM yyyy`), and a custom font that cannot be found is named in the log instead of
  silently falling back.
- Effect chains that use copy passes (a pass with no material), `#require` module directives, engine buffers such as
  `_alias_lightCookie`, or JSON with trailing commas now load instead of being skipped. The fluid-simulation Gengar
  wallpaper renders its full effect stack.
- Effects can now sample **another layer's rendered image** (`_rt_imageLayerComposite_*`), including layers that are
  hidden in the scene. Shadow Keep's moon and the pixel-cyberpunk xray scene render their missing content instead of
  a blurry or black area.

### Stability

- **Crash watchdog**: a renderer that dies unexpectedly is restarted. After three restarts in five minutes the screen
  is left idle with a "stopped after repeated crashes" note in the playing bar, instead of a restart loop.

### Documentation

- README rebuilt around what a visitor needs first: a one-command install, a contents bar, a measured performance
  table, a FAQ, supported-desktop and keyboard-shortcut lists, and the packaging routes that actually exist today.
  Fixed two claims that were no longer true (the "View log" path and the video CPU figure).
- New `SECURITY.md` (private vulnerability reporting, scope, what is not a vulnerability) and a short
  `CODE_OF_CONDUCT.md`; issue forms now ask for `wallforge --doctor` and **Copy diagnostics** output instead of a
  free-form log paste.

- README: "Performance and battery" section; renderer README documents `--scale`.
- `.coderabbit.yaml` so pull requests are reviewed by CodeRabbit with project-specific hints.

## 0.5.0

The app is now called **WallForge**. Commands, paths and the app id are unchanged.

### New

- **Share a wallpaper**: a share button in the wallpaper panel and on Discover cards copies the Steam Workshop link.
  Paste a Workshop link or id into the Discover search box to open that wallpaper.
- **Import from Wallpaper Engine**: Settings > Wallpaper Engine reads Wallpaper Engine's `config.json` and brings
  over each wallpaper's customised properties and volume, the frame rate limit and the fullscreen pause rule. Also
  `wallforge.sh --import-we [FILE] [--dry-run]`.
- **Web wallpapers** play in WebKit on the wallpaper layer, with their properties (needs `webkitgtk-6.0` and
  `gtk4-layer-shell`).
- **Volume per wallpaper**, scaled by the global volume.

### Scene renderer

- Particle systems (rain, snow, petals, sparks, dust).
- Embedded video layers play instead of being dropped.
- Text layers, including common clock and date texts.
- Effect shaders that rely on HLSL-style implicit conversions now compile: int/float mixing, vector truncation,
  float array indices, float `%`, mismatched varying widths, mismatched `mix` arguments.
- Effect passes bind the buffers the effect asks for. This fixes wallpapers that rendered blown out or washed to white.
- Layer `alignment` is honoured, which fixes layers that sat half off-screen and left black areas.
- Effects that move vertices (foliage sway and similar) draw the whole layer instead of a strip.
- Samplers fall back to their default texture when nothing is bound.
- Sprite-sheet scenes (GIF-template wallpapers) render the right frame at the right size, including textures whose
  header size disagrees with the stored mip.
- "Disable particles" in Settings now applies to the native renderer.

### Documentation

- README rebuilt around what a visitor needs first: a one-command install, a contents bar, a measured performance
  table, a FAQ, supported-desktop and keyboard-shortcut lists, and the packaging routes that actually exist today.
  Fixed two claims that were no longer true (the "View log" path and the video CPU figure).
- New `SECURITY.md` (private vulnerability reporting, scope, what is not a vulnerability) and a short
  `CODE_OF_CONDUCT.md`; issue forms now ask for `wallforge --doctor` and **Copy diagnostics** output instead of a
  free-form log paste.

- README rewritten; new guides for compatibility, importing, troubleshooting and contributing; issue templates.
- `render/tools/compare.py` builds contact sheets of renders against the wallpapers' previews.

## 0.4.0

- Scene wallpapers run on the native renderer; `linux-wallpaperengine` is no longer required, only Wallpaper
  Engine's assets.
- Auto scaling is the default: best fit per wallpaper and screen.
- Ultrawide wallpapers crop to fill 16:9 screens by default.
- Palette sync uses a sharp screen-sized render of the wallpaper.
- Clearer messages while the renderer is building or cannot be built.

## 0.3.0

- Native renderer (beta) for video wallpapers: Wayland layer-shell, VAAPI zero-copy decode, audio.

## 0.2.0

- First release: library, property editing, Discover tab, per-monitor apply, update checker, AUR packaging.
