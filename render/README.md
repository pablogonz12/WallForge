# wallforge-render

Standalone, low-overhead Wayland wallpaper renderer (wlr-layer-shell + EGL/GLES3) for Wallpaper Engine **video**
and **scene** wallpapers. WallForge starts one process per output; it can also be run by hand.

```
wallforge-render --output DP-2 --bg <wallpaper dir | video file> \
  [--scaling fill|fit|stretch] [--fps 30] [--layer background|bottom|top|overlay] \
  [--silent] [--volume N] [--duration SECS] [--software] \
  [--assets-dir DIR] [--set-property key=value]... [--disable-particles] [--scale 100]      (scenes)
wallforge-render --check <wallpaper dir | video file>                           (JSON report, no window)
wallforge-render --bg DIR --screenshot out.png [--screenshot-size WxH] [--screenshot-time SECS]   (headless)
```

Web wallpapers are not handled here: they run in `wallforge/webplayer.py` (WebKit), see the end of this file.

## Video wallpapers

- Decode: VAAPI hardware decode, frames exported as DMA-BUF (DRM PRIME) and imported zero-copy into EGL
  (`EGL_EXT_image_dma_buf_import`): Y as R8 and UV as GR88 textures, YUV->RGB in the shader. No RGBA copies.
  If VAAPI or the import fails it falls back to software decode (ffmpeg, 4 threads; Y/U/V planes uploaded as R8 textures).
  `--software` forces software decode.
- `--fps` caps presentation (default 30); extra video frames are dropped without upload/export.
- Idle wakeups: after a present the loop sleeps until the next frame is due and then reads all compositor events at once; audio writes 0.2 s chunks (0.4 s pulse buffer).
- Presentation is paced by `wl_surface.frame` callbacks: when the surface is hidden or the screen is off
  no callback arrives, so decoding stops (no timers: the loop blocks in poll until a frame is due or a signal arrives). Playback resumes where it stopped.
- Surface: all anchors, exclusive zone -1, empty input region (clicks pass through). Output resize handled.
- SIGTERM/SIGINT exit cleanly.
- Audio: ffmpeg decode -> libpulse-simple (PipeWire's pulse server), one extra thread, `--volume 0..100` (default 100), `--silent` off. Synced to the video clock with drift correction, loops with the video, paused with presentation. No audio stream = nothing started.
- Fractional scale via wp-fractional-scale-v1 + wp-viewporter (buffer in physical pixels); integer scale fallback. If the compositor goes away the renderer prints a message and exits.

## Scene wallpapers (Wallpaper Engine `"type": "scene"`)

`--bg <dir>` with a scene `project.json` takes the scene path automatically (the video path is unchanged).
Reads `scene.pkg` (PKGV) or loose files, falling back to the Wallpaper Engine assets dir
(`--assets-dir DIR`; default: Steam `wallpaper_engine/assets`, else `~/.local/share/wallforge/assets`).

Supported: image layers (transform, parenting, alpha, tint, translucent/normal/additive blending, draw order),
`--scaling fill|fit|stretch`, clear colour, `--fps` cap, user properties (`--set-property key=value`, repeatable),
.tex (RGBA8888, DXT1/3/5 kept compressed on the GPU, RG88, R8, embedded PNG/JPEG/GIF/BMP, LZ4), and multi-pass
effects: the WE shader dialect is translated to GLSL ES 3.00 with ping-pong FBOs. Static scenes render once and then
sleep; only effects using `g_Time` redraw every frame.

Also supported:
- Render scale (`--scale 25..100`): the scene is drawn into a smaller framebuffer and blitted to the surface with
  linear filtering, so a weak GPU keeps the wallpapers it likes at a lower cost. Effects keep their own resolution.
- Particle systems (`particles.rs`): box and sphere emitters, the common initializers and operators, sprite sheets,
  additive and translucent blending, one instanced draw per system, seeded and prewarmed so screenshots are
  repeatable. Trails and ropes are drawn as plain sprites. `--disable-particles` turns them off.
- Embedded video textures (a `.tex` holding an MP4): decoded in software on a thread, looped, uploaded each frame.
- Text layers (`text.rs`).
- Shader repair: ES 3.00 rejects implicit conversions the WE dialect allows (int/float mixing, vector truncation,
  float array indices, float `%`, varying width mismatches). The translator patches the source from the compiler's
  error log and retries, so such effects compile instead of being skipped.

- Audio-reactive effects: the default sink's monitor is captured (libpulse-simple, `@DEFAULT_MONITOR@`) on a thread and
  a windowed 1024-point FFT feeds `g_AudioSpectrum{16,32,64}{Left,Right}` (and the unsuffixed averages). Capture only
  starts when a loaded shader asks for those uniforms; `--no-audio-processing` turns it off. A scene whose only motion
  comes from audio still redraws when the spectrum changes.
- `--no-audio-processing` also applies to the video path's audio analysis; it is the switch behind
  Settings > Effects and power.

Not yet: SceneScript, sound objects, puppet animation, parallax,
mouse interaction, particle trails/ropes/control points. 3D scenes (perspective camera / `.mdl`) print one line and
exit with code 3. The user-facing list is in [`docs/compatibility.md`](../docs/compatibility.md).

`--check` prints a JSON report (`supported`, `unsupported`, `unsupported_layers`, `unsupported_effects`,
`missing_files`, particle and sound counts); exit code 3 when unsupported.
`--make-test-scene DIR` writes a synthetic scene used by the tests.

Debugging (environment): `WF_MAXFX=n` applies only the first n effects of each layer, `WF_ONLYFX=i` only effect i,
`WF_PASSN=n` only the first n passes of an effect, `WF_DUMP_ALL=dir` writes every translated shader to a folder.

## Tools
- `tools/batch.py` renders every installed scene headless and classifies it (works / partial / unsupported / fails).
- `tools/compare.py` builds contact sheets: render before, render after and the wallpaper's own preview.
See [`CONTRIBUTING.md`](../CONTRIBUTING.md) for the workflow.

## Build
`cargo build --release` -> `target/release/wallforge-render`
Needs ffmpeg dev libs (libav*), libEGL, libwayland-client, libva (runtime). No protocol XML needed (bundled in crates).

## Known limits
Untagged video is assumed BT.709 when height >= 720.
- Looping drains the decoder and seeks to 0; audio loops gaplessly (resampler tail flushed, pulse buffer covers the seek).
- `WF_NATIVE=1`: buffer at the video size, compositor crops/scales via wp_viewport (no power gain measured, off by default). `WALLFORGE_AUDIO_DUMP=file` writes the PCM sent to the audio server.
- Only `NV12`/`P010`-style 2-layer VAAPI exports are imported.

## Check mode
`wallforge-render --check <dir|file>` prints one JSON line without opening a window:
`ok, file, codec, profile, pix_fmt, bit_depth, width, height, fps, duration, audio{codec,channels,rate}, vaapi, vaapi_format, vaapi_error`.
On failure: `{"ok":false,"error":..}` on stdout, message on stderr, exit 1.

## Failure handling
- Bad files fail fast (decoder opens before Wayland/EGL) with a one-line error, exit 1.
- A watchdog exits 1 if one decoder call takes >6 s (corrupt streams can hang ffmpeg/VAAPI).
- Panic hook exits 1, except in the audio thread, which only loses audio.
- Env: `WALLFORGE_DEBUG` (timing logs), `WALLFORGE_FFMPEG_LOG` (ffmpeg logs).

## Web wallpapers (`wallforge/webplayer.py`)

```
python3 -m wallforge.webplayer --output DP-2 --bg <wallpaper dir> [--layer bottom|background] [--fps N]
    [--silent | --volume N] [--mouse] [--set-property key=value]...
python3 -m wallforge.webplayer --bg DIR --window 960x540 --screenshot out.png [--screenshot-time SECS]
```

A WebKit view (webkitgtk-6.0) on a gtk4-layer-shell surface anchored to one output. It loads the wallpaper's HTML
file and provides the Wallpaper Engine web API: `wallpaperPropertyListener.applyUserProperties` and
`applyGeneralProperties` are called with the wallpaper's properties (with your overrides), and
`wallpaperRegisterAudioListener` callbacks receive silence. Clicks pass through to the desktop unless `--mouse` is
given. The process is named `wallforge-web` so the app can find and stop it per output.
