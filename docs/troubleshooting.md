# Troubleshooting

## Start here: `wallforge --doctor`

One command checks the whole setup and prints what to fix:

```
$ ./wallforge.sh --doctor
WallForge 0.6.0 on Linux-...

  ok    Python 3.14.10
  ok    GTK 4.24.1               libadwaita 1.10
  ok    gtk4-layer-shell         installed
  ok    session                  niri on wayland
  ok    config                   v5 at ~/.config/wallforge/config.json
  ok    renderer                 ~/Projects/WallForge/render/target/release/wallforge-render
  ok    Wallpaper Engine assets  ~/.local/share/Steam/steamapps/common/wallpaper_engine/assets
  ok    Workshop library         122 wallpapers
  ok    video                    /usr/bin/ffmpeg, VAAPI device /dev/dri/renderD128
  ok    WebKit 6.0               installed
```

It needs no display and changes nothing, so its output can go straight into an issue. It exits 1 when something is
broken (a FAIL line) and 0 otherwise.

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
The compositor must support `wlr-layer-shell` (niri, Hyprland, Sway, river, labwc, KDE Plasma). GNOME Shell
does not, so it needs the WallForge extension: Settings > Setup > Install GNOME extension (or `./install.sh`), then log
out and back in once, because GNOME on Wayland only finds new extensions at login. Check it with
`gnome-extensions info wallforge@pgm1207.github.io` (State should be ACTIVE) and `wallforge --doctor`. Web wallpapers
do not work on GNOME yet. If another wallpaper tool draws on the same layer above WallForge, change Settings > Playback > Layer between
`bottom` and `background`.

**Web wallpapers say something is missing.**
Install `webkitgtk-6.0` and `gtk4-layer-shell` (those are the Arch package names).

## Playback

**A scene wallpaper looks different from its preview.**
Some Wallpaper Engine features are not implemented yet; [compatibility.md](compatibility.md) lists them. Check the
log for lines such as `effect ... skipped` or `unsupported`. Please report it (see below).

**A wallpaper is marked as a 3D scene and cannot be applied.**
3D scenes (models, perspective camera) are not supported.

**A visualiser is flat or does not react to music.**
Audio-reactive wallpapers listen to the system's audio output (the PipeWire/PulseAudio monitor of the default sink),
not to a microphone. Check that sound is playing through that sink, that Settings > Effects and power > "Disable audio
capture for visualisers" is off, and that the wallpaper is using the native renderer. Re-apply the wallpaper after
changing either. Silent sinks or a Pulse server that is not running leave the visualiser still.

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
WallForge's renderer stops drawing whenever the compositor says the wallpaper is not visible, to save power; it
resumes where it stopped. The "Keep playing under fullscreen apps" switch only applies to the optional
`linux-wallpaperengine` fallback.

**The wallpaper uses too much GPU, or the fans spin up.**
Lower Settings > Effects and power > Render resolution to 75% or 50% (scenes only; videos play at native size), set
the frame rate limit to 24–30, and turn off particles if you do not need them. "Reduce frame rate and resolution on
battery" does this automatically while discharging.

**The wallpaper changes by itself every so often.**
Settings > Playback > "Change wallpaper every" is on. Set it to 0 to keep one wallpaper, or turn off "Give each
screen its own wallpaper" if you want both screens to change together. `Ctrl+Shift+N` changes immediately.

**WallForge stays in the process list after I close its window.**
That is rotation: while it is on, the window closes but the app keeps running so the timer can still change the
wallpaper. Set "Change wallpaper every" to 0, or press `Ctrl+Q`, to stop it completely. Launching WallForge again
brings the window back instead of starting a second copy.

**A wallpaper stopped and the playing bar says "stopped after repeated crashes".**
The renderer died three times in five minutes and WallForge stopped retrying to avoid a restart loop. The reason is in
the log (Settings > About > View log); send it with a bug report. Pressing play on the wallpaper starts it again.

**Wallpapers are gone after logging in.**
Run `wallforge --doctor`: it shows whether the daemon runs and which login mechanisms (systemd unit, XDG entry, niri
spawn-at-startup) are in place. Turn on Settings > Desktop > Restore wallpapers at login (on by default) and open
WallForge once so it writes them. Hyprland and sway without systemd/uwsm need one line in their own config:
`exec-once = wallforge --daemon` (Hyprland) or `exec wallforge --daemon` (sway). The daemon log is
`~/.local/state/wallforge/daemon.log`; check the service with `systemctl --user status wallforge.service`.

**The wallpaper starts but another wallpaper (Noctalia, swaybg, hyprpaper) is drawn over it.**
Surfaces created later draw on top. On niri and Hyprland the daemon notices this and recreates WallForge's surface on
top within a few seconds; on other compositors start WallForge after your other wallpaper client.

**Wallpapers were lost after a dock or monitor change.**
Connector names can change between boots. WallForge saves each monitor's make/model/serial and matches by that; if two
identical monitors have no serial, they are matched by connector name.

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

Settings > Updates > **Copy diagnostics** puts the app version, system, GPU, the selected wallpaper's renderer report
and the last lines of its log on the clipboard, ready to paste into an issue.

Open an issue with:

1. The wallpaper's Workshop link (the share button copies it).
2. What is wrong, and a screenshot if you can.
3. The log file for that output, or the output of `wallforge-render --check <wallpaper folder>`.
4. Your GPU and driver, and your compositor.
