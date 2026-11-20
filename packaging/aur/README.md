# Publishing `wallforge-git` to the AUR

The package builds and packages cleanly (verified: `makepkg -f` produces `wallforge-git r109.<sha>-1`,
`makepkg --printsrcinfo` produces the `.SRCINFO` next to this file). What is left is pushing it, which needs an AUR
account with an SSH key.

## When registration is open

1. Create the account at <https://aur.archlinux.org/register> (free, no Arch membership needed). This page is behind
   an anti-bot challenge, so use a normal browser.
2. My Account → **SSH Public Key**: paste `~/.ssh/id_ed25519_aur.pub`.
3. Check the name is still free: `curl 'https://aur.archlinux.org/rpc/v5/info?arg[]=wallforge-git'` → `resultcount: 0`.
4. Push:

   ```bash
   git clone ssh://aur@aur.archlinux.org/wallforge-git.git /tmp/wallforge-git
   cp PKGBUILD .SRCINFO /tmp/wallforge-git/
   cd /tmp/wallforge-git
   git add -A && git commit -m "wallforge-git: initial upload" && git push
   ```

5. Afterwards, each release is one edit plus a push:

   ```bash
   makepkg --printsrcinfo > .SRCINFO   # regenerate whenever PKGBUILD changes
   git commit -am update && git push
   ```

## Notes

- The AUR package only carries the `PKGBUILD`; the source is cloned from GitHub at build time (`wallforge-git`).
- Binary packages for the same code are published to the pacman repository on gh-pages, built by
  `.github/workflows/arch-repo.yml`; that is what `README.md` tells Arch users to install, so the AUR package is a
  second route for people who prefer building.
- AUR registration was closed for automated-signup abuse in November 2026; the page says updates are announced on
  `aur-general` and the Arch news feed. Nothing to do but wait and retry then.
