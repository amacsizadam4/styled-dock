# Styled Dock status and roadmap

Last implementation review: **2026-11-27**, against v1.4.0 and the unreleased
working tree. Historical verification records below retain their original dates.

Styled Dock has a working dock and status-bar baseline. Full Windows taskbar
replacement remains a longer-term goal. **English and Turkish support, first-time
setup, and a Windows startup control in Settings are implemented.**
More languages and broader localization checks remain planned. Desktop reliability
and verification follow this foundation.

This is the current roadmap. Localization starts with English and Turkish and is
designed to accept more languages; the order of subsequent phases remains proposed. Future version numbers and
release dates are not assigned. Suggestions and bug reports are welcome through
GitHub issues.

## What is implemented in v1.0

"Implemented" means present in the code. It does not mean every Windows version,
hardware setup, or interaction has been verified; see the validation record below.

| Area | Implemented behavior |
| --- | --- |
| Apps and windows | Launch, pin/unpin, reorder with an animated insertion gap, detect running apps, activate/minimize, select a titled window, launch again, Close/Close all, and active/attention indicators. |
| Shortcuts and files | Start and Search default to the left with saved drag placement; optional Downloads stack, Recycle Bin, native Downloads drag-out and context menus, Show in folder, basic file drops onto apps, and executable/shortcut drops to pin. |
| Status bar | Volume and mute, Wi-Fi, battery/charging status, power plans, clock/Notification Center entry, Quick Settings, Bluetooth and display panels, and Windows system/session controls. Some controls open Windows' own UI or depend on hardware support. |
| Appearance and placement | Auto/Light/Dark themes, configurable size, icon spacing, padding, label delay, opacity, hover zoom and animation; one selected display, optional space reservation, auto-hide, and fullscreen hiding/recovery. Most settings use Save and restart. |
| Tray integration | Experimental app-icon mirroring, click/menu forwarding, an all-icons menu for overflow, and cached recovery of previously observed icons. Initial discovery remains incomplete for some apps. |
| Setup and recovery | Searchable installed-app picker, duplicate-pin feedback, atomic config saves, malformed-config backup/reset, optional launch at sign-in, unconditional single-instance protection, graceful quit, and taskbar restore commands. Windows taskbar hiding is off by default and uses a recovery watchdog when enabled. |
| Keyboard access | App menu and Quick Settings hotkeys, keyboard navigation within menus, Downloads selection/context-menu access, and native Task View and Show desktop commands. Full accessibility support is still pending. |
| Release foundation | Version 1.0.0 in Cargo and Windows metadata, local v1.0 Git tag, portable Windows x64 ZIP and checksum, usage/contributor guides, and a Windows CI workflow for builds, formatting, and selected tests. |

The titled window list, installed-app picker, startup option, instance guard,
opt-in taskbar hiding, and config recovery are already implemented. Older
development checklists listing those as future work are superseded here.

## Where verification stands

The setup/localization implementation passed `cargo test --locked --
--test-threads=1`: **35 passed, zero failures, two ignored visual helpers** on
2026-11-17. This includes language fallback, resource/placeholder coverage,
setup completion and language persistence, native Settings pages and cancel,
and startup-shortcut create/read/remove against a temporary shortcut. English and
Turkish setup and Turkish Display/Appearance layouts were inspected using the
native app. Full DPI/theme coverage and actual sign-out/sign-in startup behavior
remain to be checked. These changes are included in v1.1.0; the v1.0 release
artifacts remain a historical baseline.

The recorded pre-publication checks passed build/formatting, dock-order and
configuration tests, installed-app target resolution, shell file-transfer data
and context actions, and a duplicate-launch/graceful-quit process smoke check.

An earlier full implementation run had **29 passing tests, two failures,
and two ignored visual helpers**. The failures were native work-area reservation
and tray lifecycle tests. Tray lifecycle later passed alone; the reservation
failure reproduced alone. Earlier full runs passed. Both tests passed in the
current serial run; that single pass does not establish long-term reliability.

CI runs selected tests and does not establish full desktop integration coverage.
The earlier desktop automation helper could not target the app's windows;
Settings was targetable in the current setup/localization check. Real mouse
drag/drop, full context-menu behavior, keyboard focus, mixed-DPI monitors, sleep,
Explorer restart, and fullscreen transitions still need hands-on verification.

## Development order

### Next fix: Restore the pinned/unpinned divider

- [ ] Restore the dock divider between pinned and unpinned apps. Verify on the
  Windows desktop that it is visible at native scale when both groups are present.

### 1. Add GitHub release updates

**Implementation added to the working tree; Windows execution and desktop
verification remain pending.**

- [x] Add an **Updates** page in the application's **Settings** screen with the
  installed version, status, manual check/update actions, and release-notes link.
- [x] Persist an **Auto-update** checkbox, checked by default. Release builds
  check/install/restart on launch, at most once daily; manual actions bypass this.
- [x] Download the stable GitHub executable in the background and verify size and
  GitHub's SHA-256 digest. Independent signed-release verification remains open.
- [x] Use a helper to gracefully stop the dock/watchdog, replace the executable
  atomically, preserve config/location/working directory, and restart.
- [x] Retain a backup and require native-startup acknowledgement; attempt rollback
  when startup fails, leaving recovery details if the executable remains locked.
- [ ] Add independent release signing and signature verification.
- [ ] Verify interrupted downloads, invalid digests, unwritable folders, startup
  failure/rollback, existing pins/settings, and update/restart with taskbar hiding.
  Run the lifecycle smoke matrix and inspect English/Turkish Updates pages.

**Done when:** Windows update/recovery checks pass with settings intact and the
Updates page passes functional, keyboard, and visual checks.

### 2. Extend and verify multi-language support — English/Turkish foundation implemented

- [x] Introduce a shared localization system and translation resources, with
  English as the fallback for missing translations or unsupported languages.
- [x] Move app-owned text into translation resources: Settings, dock/status menus,
  Downloads controls, tooltips, accessibility labels, dialogs, and error messages.
  Preserve app names, file names, and text supplied by Windows or other apps.
- [x] Add an Auto (Windows language) option and an explicit language selector in
  Settings; persist the choice and define when a language change takes effect.
- [x] Start with **English and Turkish**, provide shared resources and translations
  for each, with a [contributor guide](localization.md). Register additional
  languages through the shared language registry; do not limit support to two.
- [x] Format date/time for the selected locale; support named dynamic values and
  separate singular/plural window messages for the initial languages.
- [ ] Add general plural rules and locale-aware fractional number formatting as
  languages or features require them; review right-to-left layout before adding one.
- [ ] Verify fallback behavior, saved language selection, translation coverage,
  Unicode rendering, keyboard shortcuts, and layout with longer translated labels;
  check right-to-left layout if an initial language requires it.

First-time setup opens before shell changes, preserves existing settings/pins,
and writes completion and language only on successful save. New installations use
`%LOCALAPPDATA%\StyledDock\config.toml`; existing portable files retain priority.
Settings → General includes startup at sign-in, backed by the actual per-user
Startup shortcut. The existing dock-menu startup action remains available.

**Done when:** every app-owned UI surface supports the selected language, Auto
and saved overrides work, missing translations fall back cleanly to English,
and the initial translations pass functional and visual checks.

### 3. Stabilize the existing desktop integration

**In progress — software changes included in v1.2.0.** The
[desktop validation record](desktop-validation.md) contains the environment,
repeat-test evidence, end-to-end matrix, reproduction commands, and pending
physical/interactive checks. Phase 3 is not yet complete.

The remaining manual checks are deferred at the user's request while phase 4
software work proceeds.

- [x] Handle resume without requiring the top bar, refresh device subscriptions,
  and coalesce bursts of desktop recovery notifications.
- [x] Recover a status bar receiving a stale monitor handle, and restore taskbars
  outside the watchdog's newly selected display rather than keeping them hidden.
- [x] Bound Explorer forwarding waits in the tray bridge and cover reentrant
  notifications. Preserve native work-area and tray lifecycle tests.
- [x] Add an isolated-config diagnostic for floating/reserved/auto-hide modes,
  fullscreen recovery, duplicate launch, Settings, quit, forced termination,
  taskbar restoration, and short CPU/working-set samples.

- [ ] Resolve the reproducible work-area reservation failure and investigate
  intermittent tray lifecycle failures without excluding them to get a green run.
- [ ] Verify taskbar hiding, top-bar reservations, normal quit, forced termination,
  duplicate launch, and taskbar restoration together in a disposable desktop session.
- [ ] Test Explorer restart, sleep/resume, fullscreen entry/exit, monitor
  disconnect/reconnect, and mixed DPI in floating, reserve-space, and auto-hide modes.
- [ ] Verify real pin/reorder/cancel gestures, Start/Search placement persistence,
  Downloads drag-out/context actions, app switching, and keyboard focus return.
- [ ] Test tray discovery, callbacks, overflow, and cached recovery with real apps;
  document apps or scenarios the undocumented bridge cannot support reliably.
- [ ] Check missing hardware/services, volume and power-plan changes, native sound
  output selection and older-Windows fallback; measure idle CPU/memory and long-session behavior.

**Done when:** the native failures are resolved, recovery works through the tested
lifecycle scenarios, and a recorded Windows/display test matrix supports the
documented behavior. Any remaining compatibility limits must be explicit.

### 4. Finish everyday usability and accessibility

**Software began shipping in v1.2.0; verification in progress.** See
[usability-accessibility.md](usability-accessibility.md) for usage, tests, and
explicit hardware/assistive-technology/visual limits.

- [ ] Move the Windows input-language/keyboard-layout switcher from the dock to
  the top bar. Show the active input language/layout, offer the installed layouts,
  and keep the indicator in sync with Windows and the focused app. Support mouse,
  keyboard, and accessible activation. The control and native switching are
  implemented. End-to-end checks for external changes and assistive technology
  remain open.

- [ ] Verify Mobile hotspot enable/disable on supported hardware. Its toggle,
  status, unavailable/busy reasons, and settings link live in the Network panel's
  connection section. Quick Settings links to the Network panel.
- [x] Show Ethernet, Wi-Fi, cellular, and other routed connections in the top bar
  and Quick Settings. Add links to the Windows network, Ethernet, and cellular
  settings pages in the Network panel.
- [ ] Complete screen-reader/UI Automation support for drawn dock and tray controls,
  accessible names, keyboard per-window commands, and predictable focus return.
  Native providers, keyboard navigation/context actions, and individual-window
  commands are implemented and covered by native tests. Narrator/NVDA workflow
  verification remains pending.
- [ ] Review menu spacing, anchoring, focus/selection states, long labels, tooltips,
  and offscreen behavior across the dock and status panels.
  Width measurement, full-text tooltips, native overflow/high-contrast fallback,
  and component renders are implemented; the full interactive review remains open.
- [x] Add live previews for size, zoom, and both opacity controls, with Cancel
  restoring the previous appearance; improve field-level validation feedback.
- [x] Add Styled Dock's own tray entry with Settings, Restore taskbar, and Quit;
  make the active config location clickable and improve recovery from save errors.
- [ ] Verify light/dark wallpapers, transparent backgrounds, high contrast, and
  100%, 150%, and 200% scaling across the dock, menus, and every Settings page.

**Done when:** core workflows work by mouse, keyboard, and assistive technology;
previews cancel correctly; and the visual checks show no clipping or unreadable controls.

### 5. Deepen taskbar replacement features

- [ ] Verify the unreleased Jump List implementation: Recent destinations,
  cached tasks/custom categories, and native task invocation. Pinned destinations
  and editing remain unsupported; the custom-cache reader is best effort.
- [ ] Add app progress indicators and overlay/unread badges.
- [x] Add a live window picker for apps with multiple windows.
- [ ] Verify the implemented Ctrl+Alt+Page Down/Page Up app-window cycling on
  the interactive desktop, including minimized windows and focus return.
- [ ] Complete per-window identity and relaunch handling for browser profiles and
  hosted apps. The unreleased watcher retains window AppIDs, preserves relaunch
  metadata/profile arguments in shell links, and avoids executable-only matching
  for hosted apps. Real Edge/WhatsApp and other browser-profile checks remain open.
- [ ] Reveal app windows while hovering during a file drag, improve packaged-app
  file activation, and verify complete file-drop workflows.
- [ ] Define virtual-desktop behavior and display-specific window lists before
  adding simultaneous docks on multiple monitors.
- [ ] Consider optional Win+number mapping; today those keys still use Explorer's pins.

**Done when:** each added feature has an end-to-end Windows check, and app identity,
display, and virtual-desktop rules are documented before expanding coverage.

### 6. Add plugin support

- [ ] Define the first extension points for dock items/actions and status or Quick
  Settings panels, and document which core services plugins can use.
- [ ] Choose a plugin runtime and loading model; define a versioned API, manifest,
  compatibility rules, and plugin-owned configuration before implementing a loader.
- [ ] Add plugin discovery and Settings controls to enable, disable, and configure
  extensions; report loading failures with a clear recovery path.
- [ ] Define plugin lifecycle, resource cleanup, and failure handling so a slow or
  failed extension does not block core dock interaction or taskbar recovery.
- [ ] Include localization and accessibility requirements in extension points,
  and provide a small example plugin with authoring documentation.

**Done when:** the example plugin can be discovered, configured, enabled, disabled,
and removed; compatibility and failure cases are verified; and core behavior
continues to work with plugins disabled. Runtime and distribution choices remain open.

### 7. Optimize the existing code and structure — after missing features

Use the completed feature set to guide a focused refactoring and profiling pass.
Small structural changes needed by earlier features can happen in those phases;
this dedicated pass follows taskbar features and plugin support.

- [ ] Record release-build baselines for startup, idle CPU/memory, frame times
  during hover/reorder, app-list refresh, tray updates, and plugin activity on a
  documented Windows/display setup. Identify measured bottlenecks before tuning.
- [ ] Separate the dock model and commands from native message dispatch, dragging,
  menus, and lifecycle orchestration in `src/dock/window.rs`. Keep app grouping,
  ordering, and identity rules testable without creating Windows UI.
- [ ] Split status-panel/menu orchestration in `src/topbar/mod.rs`, Settings page
  binding/validation in `src/ui/settings.rs`, and popup layout/input/animation in
  `src/ui/popup.rs` into focused modules where responsibilities are distinct.
  Keep shared localization, theme, scaling, and control behavior consistent.
- [ ] Define clear interfaces between core state, Windows services, rendering, and
  plugins. Keep Windows handle/COM ownership, callbacks, and cleanup explicit;
  centralize repeated native resource handling where that reduces lifecycle bugs.
- [ ] Profile repainting, timers, window enumeration, icon extraction/caching, and
  allocations. Reduce redundant work where measurements justify it, preserving
  the recovery checks and UI responsiveness the desktop integration relies on.
- [ ] Keep potentially slow shell, hardware, and plugin work off the UI message
  path, with bounded/coalesced requests where needed and predictable shutdown.
- [ ] Add focused regression coverage for extracted core behavior and keep native
  desktop tests visible. Compare the same scenarios before and after each change,
  document measured gains, and update the architecture guide to match the result.

**Done when:** module responsibilities and resource ownership are clear, existing
features and recovery behavior pass regression checks, and performance changes
have repeatable before/after measurements. Set numeric targets after collecting
the baseline; file size alone is not evidence of a performance problem.

### 8. Make distribution and upgrades dependable

The portable v1.0 package already exists. This phase builds on that baseline.

- [ ] Verify first launch, upgrade with existing pins/settings, and removal on a
  clean Windows profile; state the Windows versions and architectures actually tested.
- [ ] Make portable packaging and checksums repeatable, with clear release notes
  and documented support/recovery steps for each release.
- [ ] Evaluate an installer and code signing for wider distribution.
- [ ] Validate distribution packages against the GitHub updater from phase 1,
  including config preservation and recovery on a clean Windows profile.

**Done when:** a new user can install/run, configure, quit, upgrade, and remove a
documented build without losing settings or leaving Windows shell state altered.

## Ideas to revisit after daily-use feedback

Widgets, theme packs, additional per-icon visibility options, and compact clock
formats remain ideas, not scheduled work. Prioritize recurring user problems
before expanding customization.

## Keeping this roadmap current

Mark a task complete with its build/version and verification evidence. Record
partial implementation separately from missing validation. New bug reports can
change the proposed order; do not treat a phase heading as a release commitment.

Detailed implementation notes and earlier milestone proposals remain in
[development-history.md](development-history.md) as an archive.
