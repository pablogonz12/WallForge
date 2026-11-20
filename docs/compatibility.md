# Compatibility

What WallForge plays, by wallpaper type and by Wallpaper Engine feature. If a wallpaper misbehaves and the reason is
not listed here, please [report it](troubleshooting.md#reporting-a-wallpaper-that-renders-wrongly).

## By type

| Type | Player | Status |
|---|---|---|
| Video | `wallforge-render` | Works |
| Scene, 2D | `wallforge-render` | Works for most wallpapers; details below |
| Scene, 3D (models, perspective camera) | none | Not supported; detected before applying |
| Web | `wallforge.webplayer` (WebKit) | Works; audio visualisers receive silence |
| Application | none | Windows programs, not supported |
| Preset | the player of its base wallpaper | Works when the base wallpaper is installed |

## Video

| Feature | Status |
|---|---|
| H.264, HEVC, VP9, AV1 and anything else ffmpeg decodes | Yes |
| Hardware decode (VAAPI, zero-copy) with software fallback | Yes |
| Sound, volume, mute, gapless loop | Yes |
| Stops decoding while hidden or covered by a fullscreen window | Yes, always |
| Playback speed, alignment, flip | No |

## Scene

| Feature | Status |
|---|---|
| Image layers: position, scale, rotation, parenting, alignment, tint, alpha | Yes |
| Solid colour layers, fullscreen and composition layers | Yes |
| Layer blend modes | Normal, additive, multiply, screen; others draw as normal |
| Effects (multi-pass, with their buffers and masks) | Yes, including Workshop effects that bring their own shaders |
| Effects that sample another layer (render targets, `_rt_imageLayerComposite_*`) | Yes: the sampled layer is rendered first, hidden layers included. Compositing modes that depend on scene depth are still approximate |
| Textures: RGBA, DXT1/3/5, RG88, R8, embedded PNG / JPEG / GIF, sprite sheets | Yes. Sprite-sheet scenes work even when the texture header's size disagrees with the stored mip |
| GIF-template scenes (one animated sprite sheet filling the screen) | Yes |
| Embedded video layers | Yes (their sound is not played) |
| Particle systems | Sprites with the common emitters, initializers and operators. Trails and ropes draw as plain sprites; control points, child systems and vortex forces are ignored |
| Text layers | Yes. Texts driven by a clock or date script show the current time; other scripted texts show their starting value. Date/time `format` patterns (`%A`, `{year}`, Cafe-style `dd MMM yyyy`) are expanded; a missing custom font is reported in the log. Wallpaper Engine's `padding` (extra glyph geometry for text effects) is not applied |
| User properties (everything in the wallpaper's settings panel) | Yes, including image slots |
| Property-bound visibility | Yes for plain bindings and simple show/hide scripts |
| SceneScript (general scripting) | No. Scripted motion and logic stay at their starting state |
| Audio-reactive effects and visualisers | Drawn, but they receive silence (no audio capture yet) |
| Sound layers | No |
| Camera parallax, mouse interaction, cursor effects | No. Camera framing itself comes from the scene's orthographic projection and is matched; wallpapers whose previews are close-up crops (for example maps meant to span two monitors) show the same centre region Wallpaper Engine shows on one screen |
| Camera shake, camera paths | No |
| Puppet warp animation (bone-animated characters) | No; such layers are reported as unsupported |
| Lighting, reflections, 3D models | No |
| Bloom and HDR post-processing | No |

"Works" in the app means the wallpaper starts and nothing was skipped. It does not prove the picture is identical to
Wallpaper Engine. The renderer is tested against a library of real Workshop scenes by comparing each render with the
wallpaper's own preview image; differences that remain are usually one of the "No" rows above.

## Web

| Feature | Status |
|---|---|
| HTML, CSS, JavaScript, WebGL, canvas, local media | Yes (WebKitGTK) |
| User properties (`wallpaperPropertyListener.applyUserProperties`) | Yes |
| General properties (`applyGeneralProperties`: fps) | Yes |
| Audio listener (`wallpaperRegisterAudioListener`) | Registered, receives silence |
| Media integration listeners (track title, artwork, playback state) | Registered, never called |
| Mouse interaction | Off by default so clicks reach the desktop; turn off "Disable mouse interaction" in Settings to allow it |
| Pause under fullscreen windows | No |
| Chromium-only features | Depends on WebKit |

## Desktop

| | |
|---|---|
| Compositors | Any Wayland compositor with `wlr-layer-shell`: niri, Hyprland, Sway, river, labwc, KDE Plasma. Not GNOME Shell, not X11. |
| Multiple monitors | One player per output; a different wallpaper on each, or the same on all |
| Fractional scaling | Yes |
| GPU | OpenGL ES 3.0 through EGL. Tested on Mesa (AMD). |

## Wallpaper Engine settings

See [wallpaper-engine-import.md](wallpaper-engine-import.md) for what the import brings over.
