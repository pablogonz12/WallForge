# Changelog

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
- "Disable particles" in Settings now applies to the native renderer.

### Documentation

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
