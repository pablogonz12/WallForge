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
- Presentation is paced by `wl_surface.frame` callbacks: when the surface is hidden or the screen is off
  no callback arrives, so decoding stops (wakes 4x/s only to check for signals). Playback resumes where it stopped.
- Surface: all anchors, exclusive zone -1, empty input region (clicks pass through). Output resize handled.
- SIGTERM/SIGINT exit cleanly. Audio is ignored (`--silent`/`--volume` accepted).
- Only `video` projects are supported for now; other types exit with an error.

## Build
`cargo build --release` -> `target/release/wallforge-render`
Needs ffmpeg dev libs (libav*), libEGL, libwayland-client, libva (runtime). No protocol XML needed (bundled in crates).

## Known limits
- Integer output scale only (no fractional-scale). Untagged video is assumed BT.709 when height >= 720.
- Looping drains the decoder and seeks to 0 (tiny hitch possible on slow disks).
- Only `NV12`/`P010`-style 2-layer VAAPI exports are imported.
