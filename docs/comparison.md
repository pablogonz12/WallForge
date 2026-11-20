# How WallForge compares

Most people looking for "Wallpaper Engine on Linux" find one of two kinds of project: a renderer you drive from a
terminal, or a wallpaper changer for images and videos. WallForge is both a renderer and a manager in one app. This
page is here so you can pick the right tool for you, and so the honest differences are written down.

Everything below is about projects that play **Wallpaper Engine** content, not about static wallpaper changers.

## The short version

| | WallForge | [linux-wallpaperengine](https://github.com/Almamu/linux-wallpaperengine) |
|---|---|---|
| Kind | App + renderer | Renderer (CLI), GUI explicitly out of scope |
| Language | Rust renderer, Python/GTK4 app | C++ / OpenGL |
| Library, search, previews, properties | Built in | Bring your own GUI |
| Per-monitor wallpapers, rotation, favourites | Built in | — |
| Video wallpapers | Own VAAPI path, zero-copy | mpv |
| Scene wallpapers | Own renderer, see caveats below | Own renderer, longer history with the scene format |
| Web wallpapers | WebKit on the wallpaper layer | — |
| Audio-reactive visuals | Yes, from the system's output monitor | Yes |
| X11 | No | Yes |
| KDE Plasma | Yes on Plasma 6.7+ (KWin implements wlr-layer-shell) | Yes |
| GNOME Shell | No (no layer-shell; needs a Shell extension) | No, unless it draws on X11 |
| Background audio, playlist, screen-span options | Volume, mute, per-wallpaper volume | Large CLI option set |
| Needs Wallpaper Engine installed | No, it only needs the `assets/` folder (which you must own; it can fetch it from Steam) | Usually yes — it reads Steam's install folder |

If you are on X11, or you want a renderer to script yourself, use linux-wallpaperengine. If you want to browse your
Workshop library, change a wallpaper's properties, rotate them per screen and have it start at login, WallForge is
built for that.

## Where WallForge is ahead

- **One window for everything**: library with cover art, search, filters, per-wallpaper property editors, the Workshop
  browser, the settings, and the renderer. Nothing to wire together by hand.
- **Import from Wallpaper Engine**: your customised colours, sliders, combos, volumes and playback rules come over
  from `config.json` instead of being retyped.
- **Fit rules that don't stretch art**: fills the screen and crops ultrawide art, keeps portrait art whole.
- **Palette sync** with Noctalia, matugen, wallust and pywal.
- **Battery and thermal awareness**: lower frame rate and render scale on battery, optional pause, crash watchdog.
- **Discover and share**: browse the Workshop, one-click Get through your running Steam client, copy a wallpaper's
  page link.

## Where the other projects are ahead

These are real gaps, documented in [compatibility.md](compatibility.md):

- **SceneScript**: Wallpaper Engine scenes can ship a JavaScript-like script that reacts to time, mouse, media and
  other wallpapers. WallForge does not execute it. Scenes that rely on script for their look (scripted camera moves,
  media panels, triggered sequences) render incompletely here. If coverage of a specific wallpaper decides between
  the two projects, try that wallpaper with both — the renderers make different trade-offs and neither plays
  everything.
- **Sound layers**: 25 of the 77 scene wallpapers in our test library carry sound objects; they are silent.
- **3D scenes** and **puppet (mesh) animation**: not supported. 3D scenes are detected and refused rather than shown
  broken.
- **Particles** are approximated: colour and motion are close, exact emitter shapes are not.
- **Bloom and HDR** post-processing are approximated.

If a wallpaper matters to you and it looks wrong here, please
[open an issue](https://github.com/pgm1207/WallForge/issues/new/choose) with its Workshop link — the compatibility
list is driven by what people actually use.

## Other tools people use

- **waypaper**, **swww**, **swaybg**, **hyprpaper**: static images and animated GIF/WebP. Excellent at that; they do
  not play Wallpaper Engine content.
- **KDE Plasma's Wallpaper Engine plugin**: plays video and some scenes inside Plasma, KDE only.
- **Showcase galleries** such as [wpengine.alma.mu](https://wpengine.alma.mu/) show what Wallpaper Engine content
  looks like, and are a good way to find wallpapers worth subscribing to.

## Licensing and Wallpaper Engine's files

None of these projects may redistribute Wallpaper Engine's `assets/` or Workshop content. WallForge never ships
them: it uses the `assets/` folder from your own copy, or downloads it from Steam with your own account using
[DepotDownloader](https://github.com/SteamRE/DepotDownloader), and Workshop wallpapers arrive through your Steam
client when you subscribe. WallForge is MIT licensed and not affiliated with Wallpaper Engine or Valve.
