# Security policy

WallForge reads files that other people made: Wallpaper Engine scenes, their shaders and textures, web wallpapers and
their JavaScript. That makes it worth reporting quickly if something in that path is unsafe.

## Reporting a vulnerability

Use **GitHub's private vulnerability reporting**:
<https://github.com/pgm1207/WallForge/security/advisories/new> — it opens a private thread with the maintainer and
keeps the report out of the public issue tracker until there is a fix.

Please include: the version (`wallforge --doctor` prints it), the wallpaper or file type involved, what you expected and
what happened. If a proof-of-concept wallpaper file is needed, attach it as a zip.

You can expect an acknowledgement within a few days and a fix or a mitigation plan in the next release. There is no
bug bounty; fixes are credited in the release notes unless you prefer otherwise.

## Supported versions

The latest release and `main`. There are no long-term branches: fixes ship in the next release.

## What is in scope

- **The renderer** (`render/`): memory-safety or logic bugs reachable from a wallpaper's `scene.json`, shaders,
  textures, video files or `.pkg` packages, and anything that escapes the wallpaper directory.
- **The app** (`wallforge/`): command injection through wallpaper metadata (titles, properties, paths), path traversal
  in library scanning, unsafe handling of imported Wallpaper Engine settings, and the palette-sync command.
- **The updater**: anything that would let a third party push code into an installed copy, or bypass the "it still
  loads, otherwise roll back" check.
- **The asset download**: the DepotDownloader path is pinned by version and checksum; a bypass of that verification is
  a vulnerability.
- **Web wallpapers**: WebKit runs the wallpaper's own content by design, on the wallpaper layer. Escaping that sandbox,
  or a wallpaper reading files it was not given, is in scope.

## What is not

- A wallpaper rendering differently from Wallpaper Engine (that is a compatibility report — please open a normal
  issue with the Workshop link and `wallforge --doctor` output).
- Anything that needs the attacker to already own your user account, or to have write access to your config or
  wallpaper library.
- Vulnerabilities in Wallpaper Engine itself, Steam, your compositor, WebKit or Mesa: report those upstream.

## Defensive posture, briefly

The renderer is Rust with `unsafe` blocks around GL and Wayland FFI only; wallpaper files are parsed defensively
(JSON with tolerance, bounded texture sizes, no code execution from shaders beyond the shader compiler). WallForge
never runs Wallpaper Engine's SceneScript, and it never downloads executables: the only download it offers is
Wallpaper Engine's own `assets/` folder, through a pinned DepotDownloader release verified by checksum.
