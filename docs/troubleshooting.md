# Troubleshooting

## Where the logs are

Each output has its own log, overwritten every time a wallpaper starts on it:

```
$XDG_RUNTIME_DIR/wallforge/<output>.log      # usually /run/user/1000/wallforge/DP-1.log
```

When a wallpaper stops within a few seconds of starting, the app shows a toast with **View log**.

To see what the renderer thinks of a wallpaper without applying it:

```bash
render/target/release/wallforge-render --check ~/.local/share/Steam/steamapps/workshop/content/431960/<id>
```

It prints one JSON line listing layers, effects, particle systems, missing files and anything unsupported.

## Setup

**The banner says the renderer is missing or still building.**
The renderer is built on first start and by `./install.sh`. It needs Rust and clang
(Arch: `sudo pacman -S rust clang`). To build by hand: `cd render && cargo build --release`.

**"Scene wallpapers need Wallpaper Engine's assets".**
See [Wallpaper Engine's assets](../README.md#wallpaper-engines-assets). Video and web wallpapers work without them.

**The library is empty.**
WallForge reads `steamapps/workshop/content/431960` in every Steam library it finds (native, Flatpak and Snap
Steam). If Steam is somewhere unusual, start with `WALLFORGE_STEAM=/path/to/Steam ./wallforge.sh`. Press `F5` to
rescan.

**Nothing appears on the desktop.**
The compositor must support `wlr-layer-shell` (niri, Hyprland, Sway, river, labwc, KDE Plasma). GNOME Shell does
not. If another wallpaper tool draws on the same layer above WallForge, change Settings > Playback > Layer between
`bottom` and `background`.

**Web wallpapers say something is missing.**
Install `webkitgtk-6.0` and `gtk4-layer-shell` (those are the Arch package names).

## Playback

**A scene wallpaper looks different from its preview.**
Some Wallpaper Engine features are not implemented yet; [compatibility.md](compatibility.md) lists them. Check the
log for lines such as `effect ... skipped` or `unsupported`. Please report it (see below).

**A wallpaper is marked as a 3D scene and cannot be applied.**
3D scenes (models, perspective camera) are not supported.

**A clock or visualiser in a wallpaper does not move.**
Wallpaper Engine scripts (SceneScript) are not run, so anything a script animates stays at its starting value,
apart from common clock and date texts, which are recognised. Audio-reactive effects receive silence because there
is no audio capture yet.

**Video stutters or uses the CPU.**
Check the log for `vaapi`. Hardware decode needs a working VAAPI driver (`vainfo` should list the video's codec).
Without one the renderer decodes in software.

**No sound, or too loud.**
Volume is the global volume (Settings > Playback) scaled by the wallpaper's own volume (in its panel). Mute wins.
Sound from scene wallpapers is not played yet; video and web wallpapers have sound.

**The wallpaper pauses when a window is fullscreen.**
That is the default. Change it in Settings > Effects and power.

**Wallpapers are gone after logging in.**
Turn on Settings > Desktop > Restore wallpapers at login. If your compositor ignores XDG autostart entries, run
`/path/to/wallforge.sh --restore` from its startup configuration.

## Sharing and Discover

**Get does nothing.**
Steam must be running and signed in to an account that owns Wallpaper Engine. When the automatic click fails, Steam
opens the Workshop page; press Subscribe there. The wallpaper appears in the library when the download finishes.

**The share button is greyed out.**
The wallpaper is a local project that was never published to the Workshop, so it has no page to link to.

**A pasted link shows "not a Wallpaper Engine wallpaper".**
The link has to be a Steam Workshop item of Wallpaper Engine (`steamcommunity.com/sharedfiles/filedetails/?id=...`).
Items that were removed or made private cannot be looked up.

## Reporting a wallpaper that renders wrongly

Open an issue with:

1. The wallpaper's Workshop link (the share button copies it).
2. What is wrong, and a screenshot if you can.
3. The log file for that output, or the output of `wallforge-render --check <wallpaper folder>`.
4. Your GPU and driver, and your compositor.
