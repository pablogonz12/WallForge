# wallforge-render (beta)

Standalone, low-overhead Wayland wallpaper renderer (wlr-layer-shell + EGL/GLES3).
Milestone scope: Wallpaper Engine **video** wallpapers (`project.json` with `"type":"video"`, `"file":"x.mp4"`).

```
wallforge-render --output DP-2 --bg <wallpaper dir | video file> \
  [--scaling fill|fit|stretch] [--fps 30] [--layer background|bottom|top|overlay] \
  [--silent] [--volume N] [--duration SECS] [--software]
```

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
- Only `video` projects are supported for now; other types exit with an error.

## Scene wallpapers (Wallpaper Engine `"type": "scene"`)

`--bg <dir>` with a scene `project.json` takes the scene path automatically (the video path is unchanged).
Reads `scene.pkg` (PKGV) or loose files, falling back to the Wallpaper Engine assets dir
(`--assets-dir DIR`; default: Steam `wallpaper_engine/assets`, else `~/.local/share/wallforge/assets`).

Supported: image layers (transform, parenting, alpha, tint, translucent/normal/additive blending, draw order),
`--scaling fill|fit|stretch`, clear colour, `--fps` cap, user properties (`--set-property key=value`, repeatable),
.tex (RGBA8888, DXT1/3/5 kept compressed on the GPU, RG88, R8, embedded PNG/JPEG/GIF/BMP, LZ4), and multi-pass
effects: the WE shader dialect is translated to GLSL ES 3.00 with ping-pong FBOs. Static scenes render once and then
sleep; only effects using `g_Time` redraw every frame.

Not yet (stage 2): particles, audio-reactive uniforms, sound objects, puppet animation, video layers.
3D scenes (perspective camera / `.mdl`) print one line and exit with code 3. `--check` prints a JSON report
(`supported`, `unsupported`, `stage2_pending`, `missing_files`); exit code 3 when unsupported.
Known limit: ES 3.00 is strict about implicit int->float conversions the WE dialect allows; such an effect
fails to compile and is skipped with a warning. Untested against real Wallpaper Engine content so far.
`--make-test-scene DIR` writes a synthetic scene used for development.

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
