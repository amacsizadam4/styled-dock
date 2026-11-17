# How Styled Dock works

Styled Dock is a native Windows desktop application written in Rust. It uses the
`windows` crate to call Windows APIs directly. There is no web frontend or
Electron runtime. The main thread owns the windows and processes their messages;
a worker handles shell launches and icon extraction.

## Folder map

```text
src/
  main.rs             Startup, command-line options, and module wiring
  config.rs           TOML loading, defaults, validation, and atomic saves
  dock/
    window.rs         Dock input, app list, dragging, and context menus
    layout.rs         Icon positions, sizing, and hit testing
    animation.rs      Hover, launch, and reorder animation
    render.rs         Dock drawing using Direct2D/DirectComposition
    downloads.rs      Recent Downloads stack and file interactions
  topbar/
    mod.rs            Status-bar window and menus
    bar_render.rs     Status-bar drawing
    bar_status.rs     Audio, battery, power-plan, and network status
    panels.rs         Wi-Fi, Bluetooth, and display controls
  ui/
    settings.rs       Settings window
    app_picker.rs     Searchable Windows installed-app picker
    popup.rs          Shared menus, panels, sliders, and fonts
    theme.rs          Light/dark mode
    icons.rs          Vector control artwork
  platform/
    watcher.rs        Finds app windows and watches desktop events
    shell.rs          Launches apps/files and extracts Windows icons
    files.rs          Native file context menus and drag data
    placement.rs      Monitor choice, fullscreen detection, AppBar reservations
    taskbar.rs        Taskbar hiding and the independent recovery watchdog
    tray.rs           Notification-icon bridge
    tray/cache.rs     Tray snapshot validation and persistence
    lifecycle.rs      Single-instance protection and graceful shutdown
    startup.rs        Optional Startup-folder shortcut
    system.rs         Windows shortcuts and session/power actions
assets/topbar/        SVG status icons embedded at build time
resources/            Windows manifest, dialog layouts, and version metadata
examples/             Small Windows integration diagnostics
docs/                 Usage, architecture, roadmap, and development history
.github/workflows/    Windows build and selected deterministic tests
```

`main.rs` exposes shared modules through crate-level imports so modules can refer
to services consistently, such as `crate::shell` and `crate::config`.

## Launching and switching apps

The window watcher enumerates visible app windows, reads executable paths and
available app identities, and groups them into dock entries. Clicking a running
entry activates or minimizes a window. Right-clicking lists individual windows.
Pinned entries are stored in TOML. Windows shell APIs resolve icons and launch
desktop shortcuts or packaged apps.

## Drawing and input

The dock and top bar are custom Windows windows. Direct2D/DirectComposition draw
the surfaces; Windows messages carry pointer, keyboard, timer, and display events.
Layout code is separate from animation and drawing so position calculations can
be tested without rendering the entire desktop.

Downloads uses Windows-provided file icons and shell data objects. Dragging a
file out hands the transfer to Windows, which handles copy/move/link behavior.
The right-click menu comes from the shell, with an extra Show in folder action.

## Replacing the taskbar

Taskbar hiding is optional. A separate watchdog process keeps the selected
display's Explorer taskbar hidden and restores its saved state when the dock
exits or crashes. AppBar APIs reserve screen space for the top bar or dock when
requested. Explorer's auto-hide setting is global, which limits per-monitor
behavior.

Tray mirroring is the most experimental part. A bridge receives notification-icon
messages using an undocumented shell protocol, copies icon images, forwards
callbacks to their apps, and caches observed registrations. Apps that do not
register again may not appear on initial discovery. This is not a complete
replacement implementation of Explorer's shell interfaces.

## Configuration and tests

The app first looks for `config.toml` in the working directory, then beside its
executable. If neither exists, it uses the embedded `config.example.toml`.
Writes preserve existing values/comments where possible and replace the file
atomically. Invalid configuration offers backup and reset.

Tests live next to the relevant Rust code under `#[cfg(test)]`. Native integration
tests need Windows and can depend on desktop/Explorer state. The CI workflow
builds the app and runs selected tests that do not require an interactive desktop.
