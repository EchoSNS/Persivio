# ADR-027: One surface, own title bar, styled installer

- **Status:** Accepted (changes ADR-024)
- **Date:** 2026-11-25

## Context
The glass panels made the window look like boxes in a box, under a system title bar that did
not match. The installer was NSIS's default look with NSIS's icon. After an update, Windows
kept showing the previous icon in the taskbar.

## Decision
1. **One surface.** Space, headings and hairlines separate things. Glass is only used for what
   floats: dialogs, menus, messages. The solid surface setting stays.
2. **Own title bar on Windows** (`decorations: false` in `tauri.windows.conf.json`; the top
   strip is a drag region). macOS and Linux keep the system's: their conventions differ, and
   nobody has tried it there. The page asks the window (`isDecorated`) instead of guessing
   from the system.
   - Seven window permissions were added, each for one button or gesture.
   - A modal dialog makes the rest of the page inert, so every modal contains the window
     buttons and a drag strip of its own.
   - Windows 11's layout menu on the maximize button is lost; Win+Z, Win+arrows and dragging
     to an edge work.
3. **Installer: NSIS, restyled through Tauri's hook file** (`installer/persivio.nsh`). It
   replaces the page macros only; installing, updating and removing stay Tauri's code.
   `!macroundef` makes a build fail if an upgrade renames one. Buttons and check boxes stay
   real Windows controls (keyboard, screen readers) and are only painted differently.
   - Rejected: forking Tauri's template (1000 lines to merge at every upgrade) and a separate
     installer program with a web interface (needs WebView2 before it is installed, is twice
     as large, and unsigned self-extracting programs alarm virus scanners).
   - Updating by hand installs over the old version, as the updater does. Tauri's "uninstall
     first?" page is skipped. An MSI installation is removed first, after asking.
   - Silent and passive runs show the same pages or none, as before.
4. **Taskbar icon**: the installer saves the Start menu and desktop shortcuts again after
   every install. Windows shows the icon of the shortcut with the app's id and remembers it;
   neither a new program file, a new window icon nor notifying the shell changes that. Found
   by trying each on an affected computer.
5. **The MSI stays** for IT departments, described as such. Windows Installer draws its own
   pages; it only gets our images.

## Consequences
- Windows 10 shows the installer's title bar dark, not in the page's color.
- The installer's pages are English and always dark.
- Pictures are bitmaps made from `app-icon.svg` and the app's colors; a change of either means
  making them again.
