# Styled Dock

A native Rust dock and status bar for Windows. Build with `cargo build --locked`;
the debug executable is `target/debug/styled-dock.exe`.

Copy `config.example.toml` to `config.toml` in the working directory or beside the
executable. The working-directory copy takes priority. Windows taskbar hiding is
off on a fresh configuration; enable it in Settings after checking the dock.

## Everyday controls

- Start and Search appear at the far left by default. Drag either to any slot;
  their positions save with the app order. Hiding a shortcut retains its position.
- Click an app to activate/minimize it. Right-click to select an individual window,
  launch another instance, pin/unpin, or close its windows gracefully.
- Right-click the dock for Settings, Task View, Show desktop, input-language
  switching, launch-at-sign-in, or Quit. Quitting restores the taskbar watchdog's
  saved state. The restore-and-quit action also exits the dock.
- Ctrl+Alt+D opens the app menu; Ctrl+Alt+S opens Quick Settings. Use arrows,
  Tab, Enter, and Escape within menus. Shortcuts already claimed by another app
  are left alone. These are additional dock shortcuts; native Win+number still
  refers to Explorer's pins.
- Quick Settings → All app tray icons lists mirrored icons, including those that
  do not fit in the top bar. Select an icon, then Open or Show app menu.
- Sound → Choose sound output opens Windows 11's native output selector using
  Win+Ctrl+V. Sound Settings remains available on other versions.
- Settings → Pinned Apps → Installed apps opens a searchable Windows app picker.
  Choose an app, then Add (or Update selected), and Save and restart.
- Malformed configuration offers backup and reset; refusing leaves it untouched.

## Downloads

Open the Downloads stack to see recently created, completed files. Click a file
to open it. Hold its row and drag outside the stack to a folder or compatible app.
Windows handles copy/move/link modifiers and Escape cancellation. The Downloads
folder row itself is not draggable.

Right-click a file for its Windows context menu (Open, Copy, Rename, Delete,
Properties and installed shell actions as available), plus **Show in folder**.
The keyboard Menu key opens the selected item's context menu. Show in folder
opens Explorer and selects that exact file. File changes are reflected when the
stack is reopened.

Basic file drops onto dock apps pass the selected paths to the launcher.
Dropping `.exe` or `.lnk` files on the dock background pins them. Packaged app
file activation and hovering over a dock icon to reveal its window need further
integration testing.

## Recovery

`styled-dock.exe --settings` opens standalone Settings.
`styled-dock.exe --quit` requests a graceful exit.
`styled-dock.exe --restore-taskbar` exits the dock before showing Explorer's
taskbars, so the watchdog cannot immediately hide them again.

## Remaining replacement work

See [the roadmap](roadmap.md) for the complete parity checklist. This build does not yet
implement full UI Automation for drawn dock icons, Win+number remapping, native
Jump Lists, progress/overlay badges, window thumbnails, or simultaneous docks on
multiple monitors. Tray mirroring still uses an undocumented Windows protocol;
initial discovery depends on app registration. Taskbar suppression targets the
dock's selected display; Explorer's auto-hide preference remains global.
