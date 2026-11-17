# v1.0 status and roadmap

v1.0 is the first public version of this personal, AI-assisted project. It is
usable as a dock, but full Windows taskbar replacement is still a work in progress.
Suggestions and feature requests are welcome through GitHub issues.

## Included

- App launching, pinning/reordering, running-app detection, and window selection.
- Start/Search at the left by default, with saved drag placement.
- Recent Downloads with native file dragging, context menus, and Show in folder.
- Status bar, quick settings, tray mirroring, light/dark appearance, and auto-hide.
- Quit/recovery controls, instance guard, optional startup, and config recovery.
- Keyboard app/status menus and native desktop/system-control entry points.

## Known gaps

- Full screen-reader/UI Automation support and keyboard per-window commands.
- Native Jump Lists, progress bars, unread/overlay badges, and window thumbnails.
- Multiple simultaneous docks, richer virtual-desktop behavior, and complete
  per-window identity/relaunch support for browser profiles and hosted apps.
- Reliable tray discovery across all apps. The current protocol is undocumented.
- Hovering over an icon during a file drag to reveal its window, and reliable
  packaged-app file activation. Existing external drops provide basic launching.
- A dedicated own tray entry and more complete live settings previews.
- Installer, signing, automatic updates, and broad Windows/hardware testing.

## Verification limits

Build and formatting checks pass on the development machine. Dock order,
configuration, installed-app targets, file drag data/context actions, and a
duplicate-launch/graceful-quit smoke check have passed.

The latest full pre-publication implementation run had 29 passing tests, two
failures, and two ignored visual helpers. The failures were native work-area
reservation and tray lifecycle tests. Tray lifecycle subsequently passed alone;
the reservation failure also reproduced alone. Earlier complete runs passed.
These limitations remain visible rather than being hidden by CI exclusions.

The desktop automation helper could not target the app's windows, so real mouse
drag/drop, full context-menu behavior, keyboard focus, mixed-DPI monitors, sleep,
Explorer restart, and fullscreen transitions still need hands-on verification.

Detailed implementation notes are retained in
[development-history.md](development-history.md). They are a historical record,
not a statement that every mentioned workflow is verified.
