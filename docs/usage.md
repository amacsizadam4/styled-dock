# Styled Dock

A native Rust dock and status bar for Windows. Build with `cargo build --locked`;
the debug executable is `target/debug/styled-dock.exe`.

First launch opens setup before the dock changes Windows shell state. Choose
**Auto (Windows)**, **English**, or **Türkçe**, optionally enable startup at sign-in,
and review the display, appearance, advanced layout, shortcuts, and pinned-app pages.
**Finish setup** saves your choices and starts the dock. Cancel closes setup
without saving; it appears again next time. Upgrading an older config opens setup
once and preserves its settings and pins.

New installations save to `%LOCALAPPDATA%\StyledDock\config.toml`. A `config.toml`
in the working directory takes priority, followed by one beside the executable,
then the local user file. You can copy `config.example.toml` to either portable
location. Relative paths resolve beside the selected config file. The Settings
footer shows that file's location. Windows taskbar hiding is off by default.

Settings → General lets you change language and **Start automatically when I sign
in to Windows**. Language previews immediately in Settings; **Save and restart**
applies it across the dock and status bar. Cancel restores the previous language.
Auto uses the Windows user locale, falling back to English for unsupported
languages. Windows-owned menus, app names, file names, and system diagnostics keep
their original text. More languages can be added through the shared resources;
see [the localization guide](localization.md).

Startup uses a per-user `Styled Dock.lnk` in the Windows Startup folder and needs
no administrator rights. It targets this executable and opens from the selected
configuration's directory. Disabling startup removes the shortcut. Keep a portable
executable in its final location before enabling startup; re-enable it if you move it.

## Everyday controls

- Click an app with multiple windows to show their previews. Select a preview to
  focus or restore that window. Use the arrow keys and Enter to switch, or Esc
  to dismiss the picker. Windows without a preview can be selected by title.
- Ctrl+Alt+F focuses the dock; Ctrl+Alt+B focuses the status bar. Navigate with
  arrows, Tab, Home/End, activate with Enter/Space, or open the focused item's
  menu with the Menu key/Shift+F10. Escape returns to the previous window.
- Open the top-bar network icon or the network row in Quick Settings to see
  Ethernet, Wi-Fi, or cellular link status and open the relevant Windows
  settings. The Network panel contains the Mobile hotspot toggle and
  configuration link, including when the current connection is wired.
- Select the short language label next to the clock to switch the focused app's
  keyboard layout. Hover over it for the full layout name. The menu marks the
  active layout and follows input changes made with Win+Space.
- **Settings → Appearance** previews Auto (Windows), Light, Dark, and the dock
  size, hover zoom, and opacity sliders. Cancel restores the original appearance.
  **Advanced** holds exact icon sizes, spacing, and animation timing.
  The active config path opens Explorer, and Save a copy exports a recovery draft
  without changing the active file. Invalid fields are focused when saving.
- Styled Dock's own notification icon offers Settings, Restore taskbar and quit,
  and Quit independently of app-icon mirroring.

See [the phase-3 guide](usability-accessibility.md) for accessibility behavior and
the remaining validation limits.

- Start and Search appear at the far left by default. Drag either to any slot;
  their positions save with the app order. Hiding a shortcut retains its position.
- Click an app to activate/minimize it. Right-click to select an individual window,
  launch another instance, pin/unpin, or close its windows gracefully.
- Right-click the dock for Settings, Task View, Show desktop, launch-at-sign-in,
  or Quit. Quitting restores the taskbar watchdog's
  saved state. The restore-and-quit action also exits the dock.
- Ctrl+Alt+D opens the app menu; Ctrl+Alt+S opens Quick Settings. Use arrows,
  Tab, Enter, and Escape within menus. Shortcuts already claimed by another app
  are left alone. These are additional dock shortcuts; native Win+number still
  refers to Explorer's pins.
- Quick Settings puts brightness (on supported displays) and volume sliders
  beside network, Bluetooth, and power controls. Enable **Show app tray icons**
  in Settings → Display to mirror app icons and show their list in Quick Settings.
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
Jump Lists, progress/overlay badges, or simultaneous docks on
multiple monitors. Tray mirroring still uses an undocumented Windows protocol;
initial discovery depends on app registration. Taskbar suppression targets the
dock's selected display; Explorer's auto-hide preference remains global.

## Updates

Settings → Updates shows the installed version, update status, a release-notes
link, **Check for updates**, and **Update and restart**. **Auto-update** is enabled
by default. Save and restart to apply changes to this preference.

Release builds attempt an automatic update on launch, at most once per day.
Manual checks and updates bypass that interval. Updates use the latest stable
GitHub release; development builds disable downloads and replacement.

The helper downloads `styled-dock.exe`, checks its size and GitHub SHA-256 digest,
then gracefully closes the dock and waits for its taskbar watchdog. It replaces
the executable atomically, restarts with the same working directory, and requires
a native-startup acknowledgement. Settings and pins are not replaced. A failed
startup triggers a graceful shutdown and restoration from the backup when the
executable is no longer in use. If recovery is blocked, the backup remains beside
the executable as `.update-backup` for manual recovery.

Updates require write access to the executable's folder. Protected installations
must be updated manually; the updater does not request administrator privileges.
Status and failure details live under `%LOCALAPPDATA%\StyledDock\Updates`.
The digest verifies integrity against GitHub's metadata; this currently trusts
the repository and HTTPS, and is not independent release-signature verification.
