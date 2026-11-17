> Archived development notes. The current public status is in [roadmap.md](roadmap.md).

# Styled Dock polish roadmap

**Current milestone** · M1 — UI and interaction polish
**Target** · A Windows dock and menu bar that feel intentional, stay readable over any desktop, and are safe to use every day.
**Order** · Finish the current polish pass, harden desktop integration, improve daily workflows, then prepare a distributable build.

## Baseline

2026-11-17: Replacement essentials implementation pass. Added native Downloads
drag-out and shell context menus with Show in folder, including Menu-key access;
shell tests verify file-transfer data and native context actions. Start/Search
now default to the far left, can be dragged across the dock, and save positions
atomically with pin order; regressions cover visibility, movement/refresh and
configuration round-trip. Added unconditional single-instance guarding, graceful
quit/restore commands, opt-in taskbar hiding on fresh setup, selected-display
taskbar suppression, Close/Close all, active/attention indicators, keyboard app
and Quick Settings menus, all-tray-icons access, searchable installed-app picker,
early duplicate-pin feedback, launch-at-sign-in, config backup/reset recovery,
basic external file drops, Task View/desktop/input-language commands, and a native
Windows 11 sound-output selector entry. Per-window registered App IDs are read
when they resolve to launchable AppsFolder items; unregistered identities still
fall back to the existing process grouping. See README.md for controls.

Validation: debug build and formatting pass. All five dock interaction tests,
both config tests, installed-app target resolution and shell transfer/menu tests
pass. A full run before the shortcut-position change passed 30 tests, with two
visual helpers ignored. The latest full run passed 29, failed two existing native
tests (top-bar work-area reservation and tray lifecycle), and ignored two. The
work-area failure reproduces in isolation. Desktop automation could not expose
the app's windows even after launch; real drag/drop, context-menu commands,
keyboard focus, and multi-display behavior remain unverified. No full taskbar
parity claim is made.

Follow-up validation: tray lifecycle passes in isolation; the work-area test
continues to fail alone. A real-process smoke check with an isolated config
verified that a duplicate launch exits while the original remains alive, and
`--quit` exits both the command and original instance with success. Corrected a
shutdown race where an owned Settings window could disappear before the quit
command posted its second close. Tray menus snapshot keys/names before pumping
messages, avoiding a borrowed tray-state panic when an app updates its icon.

### Replacement parity follow-through

- [x] Downloads file drag-out and native context actions, including Show in folder.
- [x] Start/Search at the left by default, with persistent drag placement.
- [x] Discoverable dock-menu Quit/Restore and CLI recovery; unconditional instance guard.
- [x] Basic keyboard app/status menus; native desktop/task/input-language entry points.
- [x] Foreground and attention indicators; Close/Close all.
- [x] Access to all mirrored tray icons when the bar fills.
- [x] Searchable installed-app picker, startup option, config backup/reset.
- [x] Basic external file drops and sound-output selector entry.
- [ ] Full screen-reader/UI Automation support for drawn icons and tray controls.
- [ ] Keyboard per-window commands, predictable focus return, optional Win+number mapping.
- [ ] OLE hover-to-reveal, packaged-app file activation, and end-to-end file operation tests.
- [ ] Full per-window AppID/relaunch handling, browser-profile and hosted-app coverage.
- [ ] Native Jump Lists, recent documents/tasks, progress bars and overlay/unread badges.
- [ ] Window thumbnails and richer multiwindow activation/cycling.
- [ ] Multiple simultaneous docks, display-specific window lists and virtual-desktop policy.
- [ ] Complete initial tray discovery and lifecycle compatibility across real apps.
- [ ] Live native sound-output switching verification and older-Windows fallback behavior.
- [ ] Resolve intermittent shell reservation/tray regressions and complete desktop visual QA.
- [ ] Dedicated own tray entry, clickable config location, release packaging and clean-profile checks.

2026-11-12: Added fixed Start menu and Windows Search dock buttons, routed through native Win and Win+S input on the shell worker. Settings now has a Shortcuts page with independent Start, Search, and Downloads visibility controls; all default on, old configs receive defaults, and Save and restart persists the choices in [dock]. The shortcuts use vector artwork and stay outside app pinning/reordering. All eight visibility combinations, existing drag behavior, configuration defaults/save round-trip, and every Settings page pass. Full suite: 25 passed, three existing native tray/work-area tests failed in this desktop session, two visual helpers ignored; the three failures also reproduce in isolation. Formatting and debug build pass; prepared executable plus a copy of the current configuration under target/shortcuts/debug because the standard executable is running. Live Start/Search click-through remains unverified.

2026-11-12: Fixed the hidden Windows taskbar leaving its work-area reservation behind. The independent watchdog now saves Explorer's taskbar state, temporarily enables auto-hide to release that band, continues hiding the taskbar windows, and restores the original preference on normal exit or parent-process termination. Repeated enforcement repairs a lost auto-hide state without overwriting the saved preference. The lease starts before placement and the tray bridge, avoiding a startup deadlock when the bridge intercepts shell API calls, and outlives the bridge on shutdown. A native regression failed before the fix and now covers the released bottom band, preserved top reservation, already-enabled auto-hide, recovery, and restoration. Full suite: 27 passed, two visual helpers ignored; debug build and formatting pass. Live checks with the standard debug build verified work area y=32..1080 and dock bottom y=1080 on the 1920x1080 monitor, restoration from state 1 to the original state 0 after both normal exit and forced parent termination, and stable geometry through repeated tray discovery. Relaunched with the existing configuration. The inspect_topbar helper now includes monitor/dock bounds and supports --once.

2026-11-12: Fixed the remaining transient topbar work-area flicker at its source. Tray discovery excluded Explorer and the bridge but still sent synthetic TaskbarCreated to StyledDockWindow; that window queued WM_SHELL_RESTART, and AppBar::reregister removed/recreated the reservation, briefly returning rcWork.top to zero. The earlier loss-episode latch reduced frequency without preventing this path. Discovery now also excludes this process's dock and topbar windows while preserving app registrations and genuine shell notifications. A native recipient regression failed before the fix (the dock received one false restart) and passes afterward while an app recipient still receives its request. All 26 automated tests pass, two visual helpers remain ignored; debug build and formatting pass. Relaunched the standard debug build with its existing config. Live 25-ms geometry sampling through startup plus forced tray discovery held the 32-pixel reservation with no transitions, compared with multiple 0↔32 flips before the fix. The inspect_topbar example now supports --refresh-tray to repeat six discovery requests during a ten-second sample.

2026-11-11: Traced recurring topbar spacing flips to tray priority recovery. Every one-second lookup-priority loss requested TaskbarCreated again; real apps deleted/re-added their icons in varying reply order while the monitor work-area top alternated between 0 and 40 pixels. Recovery now requests registration once per continuous loss episode, retaining bounded startup retries and ordinary lookup-order repair. Registration requests exclude Explorer and the bridge itself. Extended the native priority-recovery regression to verify five consecutive losses cause no repeated announcements. Debug build/formatting pass; 24 tests passed, two visual helpers ignored, and the existing AppBar/tray work-area test failed intermittently in the full run but passed in isolation. Relaunched the standard debug build with existing config: bounded startup registration still briefly recalculates the work area; a subsequent ten-second live sample held rcWork.top=40 and fixed bar geometry without a single transition.

2026-11-11: Fixed top-bar spacing with the tray bridge active. The bridge takes Shell_TrayWnd lookup priority, so it also intercepts SHAppBarMessage. Forwarding those packets unchanged left query/set reply memory addressed to the bridge instead of Explorer: calls could report success without reserving any space. The bridge now allocates reply memory for Explorer and copies the result back before returning. The new native regression failed before the fix and now verifies rcWork, repeated hide/show and re-registration, and a real maximized window's visible frame below the reserved band. Full suite: 25 passed, two interactive helpers ignored. Formatting and debug build pass. After the old process exited, rebuilt and launched the normal target/debug executable with the existing configuration. Live native diagnostics confirm the top bar occupies y=0..32 and the monitor work area begins at y=32, with its taskbar watchdog running. A second prepared build is available under target/topbar-fix/debug.

2026-11-11: Dock dragging now previews a moving gap with critically damped horizontal motion; the held icon follows the pointer above its neighbours, captures the mouse, and settles into place on release. Running apps dropped to the left of the pinned divider become saved pins, including when the pin list is empty; fixed shortcuts, the pin limit, cancellation outside the dock, and persistence rollback are protected. Downloads now opens a vertical curved fan of up to seven recently added files, with Windows file icons, staggered opening/reverse closing, keyboard selection, Escape/outside dismissal, an empty-folder state, and Open Downloads. Partial downloads are excluded and animation-off settings are respected. Its nested message loop preserves pending dock/tray events. Tray startup now takes lookup priority before a deferred registration broadcast, retries during startup, requests missed registrations after priority loss or Explorer restart, and permits notification IPC from lower-integrity apps. A launched build collected 13 real app tray entries. Formatting and debug build pass; 24 tests pass, two existing visual helpers are ignored. New native regressions cover tray startup/priority recovery and fan selection after idle with animations enabled/disabled. Real mouse drag and fan visual checks remain outstanding because the desktop automation tool did not expose the dock windows.

2026-11-11: Hardened auto-hide recovery after fullscreen video. A 200 ms UI timer rechecks foreground window geometry when transition events arrive early or are missed; fullscreen detection no longer relies on global notification-suppression state. Returning from fullscreen explicitly restores the bars' topmost order without activation. Auto-hide stays on the physical monitor bottom and polls its reveal strip, initializing hover tracking even for a stationary pointer. The independent taskbar watchdog now re-hides Explorer on a 200 ms timeout as well as explicit refresh requests. Five new native/geometry regressions pass, covering repeated fullscreen resize recovery, nonactivating show, reveal-edge boundaries, physical-edge placement, and watchdog timeout/exit handling. Full suite: 18 passed, two interactive helpers ignored, one existing AppBar reservation test failed at ABM_NEW registration in the current session. A subsequent run also saw an intermittent failure in the existing native tray lifecycle test at NIM_MODIFY. Build and formatting pass. Live browser/video reproduction remains unverified. The running debug executable is locked, so the updated build and a copy of its current configuration are prepared under target/autohide-fix/debug.

2026-11-11: Topbar volume artwork now reflects mute, silence, and low/medium/high output; volume sliders show a matching live speaker icon, level description, and percentage. Battery status shows a proportional fill and percentage, a green charging bolt, or a plugged-in marker when connected without charging; low charge uses warning colors, desktops show a plug, and unknown levels show a dash. Clicking the date/time now requests Windows Notification Center through Win+N. Quick Settings uses shared vector outline icons with supersampled native GDI rendering, 400-DIP content width, 94-DIP card rows, and a wider column gutter. Wi-Fi/Bluetooth/Display panels and the volume panel also gained space. Native offscreen artwork was inspected in dark/light themes at 100% and 150%; debug build, formatting, and 14 automated tests pass. Two visual helpers are ignored by default. Live desktop click-through and physical battery state changes remain unverified: the computer-use helper did not expose the launched dock windows.

2026-11-11: Tray icons now survive dock restarts even when their apps ignore TaskbarCreated. The bridge saves copied icon pixels, callback mode, visibility, and ownership to an atomic per-user snapshot, flushes on restart, and restores only entries whose window and process creation time still match. Updates are batched on the existing timer. The native lifecycle test restarts the bridge without another app registration, verifies the restored image and version-4 right-click callback, and checks deletion survives another restart. Cache tests cover hidden icons, dead/reused owners, and malformed snapshots. Debug build and formatting pass; 13 tests pass, with one interactive preview ignored. First discovery of an icon still requires its app to register or update it; this snapshot recovers icons previously observed by the updated bridge.

2026-11-11: Fixed top-bar work-area recovery: a cached AppBar rectangle no longer prevents restoring a lost Windows reservation, and showing the top bar rechecks its placement. Dock auto-hide continues to affect only the dock. The native regression test releases the reservation without changing the cached rectangle, verifies recovery, and covers hide/show without growing the reserved band. Debug build and formatting pass; 11 tests pass, with one interactive preview ignored.

2026-11-11: App labels now wait one second by default and restart their delay when the hovered app changes. Settings has a Layout page for overall scale, independent icon size, icon spacing, edge padding, and label delay. Display includes an experimental app tray toggle. The top bar mirrors individual notification icons through a Shell_TrayWnd bridge, copies their actual icon images, and forwards legacy and version-4 click callbacks to their owning apps. Notification messages are also forwarded to Explorer, and apps are asked to register again after the bridge closes. This protocol is undocumented; apps that ignore TaskbarCreated may need to update their icon or restart before appearing. Nine tests pass (one interactive preview ignored), including hover timing, config round-trip, malformed tray packets, and a real Shell_NotifyIcon add/modify/hide/show/version-4 right-click/delete lifecycle. Debug build passes. Desktop visual verification remains outstanding: the computer-use helper did not expose the launched Settings/dock windows. Check actual app menus, Explorer restart, and the tray under monitor changes before treating it as stable.

The menu fade-out pump now validates pending repaints rather than starving its close timer. A native regression test selects a menu action with a dirty child window, verifies the queued Settings request survives the fade, and opens Settings afterward. Quick Settings now has a two-column Windows control grid, with dedicated Wi‑Fi, Bluetooth, and Display panels. Wi‑Fi uses native network/profile data; Bluetooth lists cached devices and offers radio control subject to Windows access; Display uses monitor brightness when supported and links to Windows night-light/display controls. Pairing, new-network credentials, and unsupported monitor brightness remain managed by Windows.

Appearance now offers Auto (Windows app mode), Light, and Dark, with a live color preview that reverts on Cancel and persists through Save. The dock groups pinned apps, unpinned running apps, then fixed Downloads and Recycle Bin shortcuts, with separators between groups. Windows supplies the folder icons and resolves Downloads through its known-folder API. Verified 2026-11-11: both Settings palettes visually checked; four tests pass, including theme parsing, group order, config preservation, and actual Windows shell icon extraction.

Styled Dock already has running app detection, pinning and reordering, hover zoom, launch animations, and configurable opacity for both bars. The menu bar has Wi‑Fi, battery, sound, date and time, and Quick Settings menus. Battery lists Windows power plans, and Settings has Display, Appearance, and Pinned Apps pages. Changes in Settings currently apply through **Save and restart**.

The top-left Windows 11 logo opens a system menu with About this PC, Task Manager, Microsoft Store, Sleep, Restart, Shut down, Lock screen, and Log out. Sleep is disabled when neither traditional sleep nor Modern Standby is available. The 2026-11-11 debug build and all three existing tests pass; session and power actions were not invoked during testing.

The Windows preview verified battery menu rendering and Escape dismissal, opening Settings from Quick Settings, and saving opacity changes. The earlier drag check covered ordering in code, and the power plan check had only one installed plan. Treat real mouse dragging and switching between plans as outstanding verification.

## M1 — Make the current UI feel finished

**Active. Next build should focus on these tasks.**

- [ ] **Menus and buttons.** Review all five status menus and the dock context menu together. Refine spacing, menu anchoring, focus and selection states, and long label handling. Give icon-only controls useful tooltips and accessible names. Make status buttons and dock actions reachable by keyboard, with Escape returning focus predictably.
- [ ] **Settings feedback.** Preview size, zoom, and both opacity sliders in the running app. Cancel should restore the previous appearance. Keep changes that affect desktop placement or shell behavior behind a deliberate save. Show validation errors beside the relevant control, including when Save is clicked from another page.
- [ ] **Dock interaction.** Test pin, unpin, and drag with real mouse input. Check that a click still launches or switches apps, a drag shows a clear drop location, and releasing outside the dock cancels cleanly. Confirm the saved order after reopening.
- [ ] **Visual pass.** Capture the menu bar, battery menu, Quick Settings, dock context menu, and each Settings page. Check light and dark wallpapers at 0% and default opacity, Windows high contrast, and 100%, 150%, and 200% display scaling.

**Done when** the core controls are usable by mouse and keyboard, previews can be canceled, and the screenshot pass shows no clipping, unreadable labels, or menus opening offscreen.

## M2 — Make it reliable as a daily driver

- [ ] **Taskbar and lifecycle.** Add a discoverable tray entry with **Settings**, **Restore Windows taskbar**, and **Quit**. Prevent a second instance from drawing a duplicate dock even when taskbar hiding is disabled. Make hiding the Windows taskbar opt-in on a fresh setup. The bundled defaults currently enable it.
- [ ] **Display changes.** Test monitor unplug and reconnect, mixed DPI, Explorer restart, sleep and resume, and entering or leaving fullscreen. Cover floating, reserve-space, and auto-hide modes. Check that the top bar releases reserved space on exit and that auto-hide works with a monitor above or below the dock display.
- [ ] **Config recovery.** Make the active config location easy to open. Give malformed or unwritable configs a recovery path with the original file preserved. Keep portable configs and existing pin lists usable across upgrades.
- [ ] **Idle behavior.** Record CPU and memory with the bars idle, compare after a longer session and repeated monitor changes, and check for duplicate bars, stranded workers, or persistent repainting.

**Done when** normal exit and forced termination restore the expected taskbar state, a second launch has a clear outcome, and the display and recovery checks pass on Windows.

## M3 — Improve the workflows people use every day

- [ ] **Choose an app window.** Add a titled window list to the dock menu when an app has multiple windows. Selecting a row should restore or focus that exact window. Keep ordinary clicks quick.
- [ ] **Pick installed apps.** Add a searchable picker for desktop and packaged apps. Keep path and App ID entry available for custom shortcuts. Catch duplicate pins and offer a way to repair a target that moved.
- [ ] **Make status menus more useful.** Sound and Quick Settings now have a live volume slider with percentage and mute controls (2026-11-11: debug build and all three existing tests pass). Verify mouse dragging and keyboard adjustment on Windows; the automation helper could not target the dock windows. Verify battery plan switching on a machine with multiple plans. Check Wi‑Fi, audio, and battery menus when the relevant hardware or service is unavailable.
- [ ] **Review visual options after daily use.** Auto, Light, and Dark themes are implemented. Consider per-icon visibility or a compact clock format after daily use. Prioritize requests that solve a recurring problem in actual use.

**Done when** switching between two windows, pinning an installed app, adjusting volume, and switching between available power plans can each be verified end to end.

## M4 — Prepare a preview release

- [ ] Publish one versioned Windows build with the app icon, version metadata, a quick start guide, and documented exit and taskbar recovery steps.
- [ ] Verify first launch, upgrade with an existing config, and removal on a clean Windows profile. State the Windows versions and architectures actually tested.
- [ ] Start with a portable download. Revisit an installer, launch at sign-in, and code signing before wider distribution.

**Done when** someone new to the project can run, configure, quit, and remove it using the guide, and an existing user can upgrade without losing pins or appearance settings.

## Keep for later

Live window thumbnails, docks on multiple monitors at once, widgets, theme packs, and automatic updates. Revisit after M2 and M3, using daily feedback to pick the next feature.

For each completed checkbox, record the build and one piece of evidence in this file, such as a screenshot, a test result, or the Windows setup used. Start the next session with **M1 menus and buttons**, then run the mouse drag check before expanding the feature list.
