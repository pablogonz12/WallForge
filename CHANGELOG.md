# Changelog

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
