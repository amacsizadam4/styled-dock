# Desktop integration validation

Phase 2 is in progress. This record separates native automated checks from real
hardware and mouse checks. A passing message simulation is not a sleep/resume,
Explorer restart, or monitor reconnect test.

## Environment and baseline — 2026-11-18

- Windows 11 Pro, build 26300, x64; one display at 120 DPI (125%).
- Source based on v1.1.0, with the phase-2 changes below included in v1.2.0.
- Existing dock stopped before native tests. Smoke checks use separate configs
  under `.test-tmp`, with setup already completed and tray cache redirected to
  an isolated directory. They do not overwrite the user's config or startup entry.
- Audio and WLAN services were running. Service outages, other Windows builds,
  and hardware changes were not induced.

The initial repeat run passed twice. Its third run failed in the Downloads
keyboard test: the pointer could hover a file while the test assumed the folder
row was still selected. Work-area reservation and tray lifecycle checks passed
in all three runs. The test now explicitly selects the folder with Home before
Enter; Home/End are supported in the Downloads menu. No tests were excluded to
obtain the subsequent results.

After the fixes, the final full suite passed **five consecutive runs**, each with
**40 passed, zero failures, two pre-existing ignored visual helpers**. This
includes native work-area and tray lifecycle tests. The historic reservation/tray
failures were not reproduced on this desktop; repeat passes do not establish
their absence on every Windows/Explorer version.

## Changes covered by regression tests

- Monitor loss: the status bar accepts a stale monitor handle and falls back to
  a valid display before the dock's display notification arrives. Its reservation
  survives and it does not take focus.
- Monitor scope: the watchdog restores taskbars outside the newly selected
  monitor, including fallback/return transitions. Reused window handles belonging
  to another process are not touched. This policy is tested with simulated display
  membership; physical disconnect/reconnect remains pending.
- Resume: the dock handles automatic, user, and critical resume notifications
  even when the top bar is disabled. It refreshes placement, app discovery, radio
  status, and audio endpoints. Missing audio/WLAN subscriptions are retried on
  later status refreshes. Native notification bursts are coalesced.
- Tray: native forwarding uses a 500 ms deadline, including AppBar shared-reply
  forwarding, so an unresponsive recipient cannot block that call indefinitely.
  The deadline and reentrant registration/timer handling have regression tests.
- Existing tests still cover top-bar reservation repair, a maximized app staying
  below the bar, fullscreen geometry recovery, reveal-strip boundaries,
  hidden/visible tray changes, version-4 callbacks, add/delete, and cache recovery.

## End-to-end matrix

The diagnostic launches the actual release executable and asserts work-area,
visibility, process, and restoration state. Tray mirroring is enabled in the
top-bar scenarios. Each scenario tests duplicate launch and Settings, then quits
or terminates the owning dock process. It verifies that the original taskbar
auto-hide state, visibility, and work area return; the recovery command is also
checked after the dock is already gone.

| Placement | Top bar | Normal quit | Forced termination | Foreground fullscreen entry/exit |
| --- | --- | --- | --- | --- |
| Floating | On | Passed | Passed | Passed |
| Reserve space | On | Passed | Passed | Passed |
| Auto-hide | On | Passed | Passed | Passed |
| Floating | Off | Passed | Not in this matrix | Passed |

Fullscreen uses a test-owned borderless foreground window. It then shrinks that
same window, verifying recovery without requiring another activation. The tool
also delivers display, settings, resume, and TaskbarCreated notifications to the
test-owned dock; it does not actually change displays, suspend Windows, or kill
Explorer. The idle sample closes Settings before measuring the dock process;
Settings is reopened before shutdown to exercise cleanup with both windows open.

Final release-build samples on this desktop were **0.00–0.62% of one CPU core**
over five seconds per scenario. Working set was **67.5–69.1 MiB** with the top bar
and tray enabled, and **54.2 MiB** with the top bar disabled. These are short local
samples after initial recovery/Settings activity, not steady-state guarantees,
comparisons against v1.1.0, or evidence that long-session memory growth is absent.

## Reproduce

### During development

Quit Styled Dock, then choose the command that matches the change. Run these
examples from the local checkout root in regular PowerShell on Windows. Use the
logged-in desktop for checks that depend on native windows or Explorer.

```powershell
.\scripts\dev.ps1 Test config::tests
.\scripts\dev.ps1 Test i18n::tests
.\scripts\dev.ps1 Test dock::window_picker::tests
.\scripts\dev.ps1 Launch
.\scripts\dev.ps1 Smoke -Fullscreen
```

Run the relevant test filter first. For a live preview, keyboard, or focus
change, launch the debug build and exercise the affected interaction on the
desktop. Quit the dock to finish the launch command. For visual evidence,
capture the affected region at native scale, and use a full desktop capture
when checking monitor placement, clipping, or the taskbar relationship.

Use Smoke for placement, fullscreen, taskbar, and lifecycle changes. It builds
the debug dock and the diagnostic, then runs the seven-scenario matrix.
`-Fullscreen` adds fullscreen transitions. Add `-Taskbar` to exercise actual
taskbar hiding on a disposable desktop. The matrix covers opening Settings
and leaves the live window picker to targeted checks.

The runner writes Cargo artifacts to `%LOCALAPPDATA%\StyledDockDev\target` and
prints a short summary with the full log path under
`%LOCALAPPDATA%\StyledDockDev\logs`. It prints the last 35 log lines when a
command fails. A test filter that runs zero passing tests fails the check.

### Release validation

Run native tests on an idle, disposable Windows desktop with the installed dock
stopped. They temporarily affect AppBar/taskbar state and show native windows.

```powershell
cargo fmt --check
cargo test --locked -- --test-threads=1
cargo build --release --locked
cargo build --locked --example desktop_smoke
.\target\debug\examples\desktop_smoke.exe --exe .\target\release\styled-dock.exe --taskbar --fullscreen
```

Without `--taskbar`, the diagnostic does not enable taskbar hiding. Without
`--fullscreen`, it does not create/foreground its fullscreen test window. The
tool refuses to run if a dock window already exists. It retains test configs
under `.test-tmp` for inspection, restores captured Explorer state on error, and
prints CPU/working-set samples. Those samples cover the dock process, not GPU
memory, Explorer, or the independent watchdog; short samples are not a benchmark
or a long-session leak test.

For repetition, run the full suite five times with `--test-threads=1`, preserving
each log. Stop and investigate the first failure; do not retry away a failure or
run simultaneous native suites against the same desktop.

## Remaining verification

| Scenario | Required observations | Status |
| --- | --- | --- |
| Real Explorer restart | Re-register both reservations; rediscover/recover icons; taskbar hide and later quit restore correctly | Pending in a disposable desktop |
| Real sleep/hibernate and resume | Correct clock, power, audio/radios; no frozen UI or lost reservations, including with top bar off | Pending |
| Display disconnect/reconnect | Move to fallback display, restore the previous taskbar, return to configured display; all three placement modes | Pending: second display required |
| Mixed DPI | 100/125/150/200% across different displays; no clipping, offscreen menus, bad hitboxes, or lost reveal strip | Only one display/125% exercised |
| Mouse and keyboard workflows | Pin, reorder, cancel, Start/Search persistence, actual Downloads drag-out/context actions, app switching, focus return | Model/native-shell checks pass; real gestures pending |
| Real app tray compatibility | Record app/version, discovery timing, open/context callbacks, overflow, cache recovery after dock and Explorer restarts | Synthetic/native Shell_NotifyIcon checks pass; app matrix pending |
| Missing devices/services | Missing battery/Wi-Fi/Bluetooth/audio; device removal/return; failed power-plan change and native sound output on older Windows | Recovery code reviewed; fault/hardware matrix pending |
| Long session | At least one hour of idle plus repeated menus/reorders; CPU, private memory and handles before/after sleep and Explorer restart | Short samples only; soak pending |
| Clean profile | Repeat shutdown/recovery matrix without the developer's existing shell/profile state | Pending |

The tray bridge still relies on an undocumented Shell_TrayWnd packet protocol.
Only the tested Windows x64 environment is covered here. Some apps ignore
registration requests or expose callback/ownership patterns that cannot be
recovered from an old cache. Cache entries require a live matching process;
restarting an app invalidates its old callback identity. The app-owned status
controls replace several Windows tray icons. None of these checks establish
universal tray discovery, 32-bit compatibility, or full taskbar parity.
The forced-termination matrix kills the dock process while its independent
watchdog remains alive; killing the watchdog or the entire process tree is not
covered by that recovery claim.

## API references

- [ABM_SETPOS](https://learn.microsoft.com/en-us/windows/win32/shell/abm-setpos):
  Explorer approves the reservation rectangle; a successful return alone does
  not replace checking the resulting work area and maximized-window geometry.
- [Resume notifications](https://learn.microsoft.com/en-us/windows/win32/power/pbt-apmresumeautomatic):
  automatic resume is distinct from the notification of resumed user activity.
- [SendMessageTimeoutW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendmessagetimeoutw):
  timeout/recipient-exit behavior used for cross-thread tray forwarding. Calls
  into a window on the same queue are synchronous and do not gain that timeout.

## Unreleased app integration — 2026-11-27

Formatting, translation-key parity, and diff whitespace checks passed.
Shortcut identity, relaunch properties, hosted-app pin grouping, native Shell
Link persistence, and malformed cache input still need native Windows testing.

To complete Windows validation, quit the dock and run these focused checks from
the local checkout root in regular PowerShell:

```powershell
cargo fmt --check
.\scripts\dev.ps1 Test platform::app_identity::tests
.\scripts\dev.ps1 Test platform::jump_list::tests
.\scripts\dev.ps1 Test platform::watcher::tests
.\scripts\dev.ps1 Test hosted_app_pins_match_the_app_identity_before_browser_paths
.\scripts\dev.ps1 Test i18n::tests
```

Then check the actual app menu's Recent and task actions and capture its region.
Cycle three windows forward/backward, including a minimized window and a dock
keyboard selection, checking focus and wraparound. On a machine with Edge web
apps, run Edge alongside WhatsApp and a second browser profile; verify separate
entries, installed-shortcut matching, pin/restart, icon/name, Launch Again, and
that closing WhatsApp leaves the main browser entry intact. These scenarios
remain pending; no real WhatsApp/Edge installation was added for this change.

## Updater working-tree checks — 2026-11-27

`git diff --check` passed. Config coverage includes the default-enabled updater
preference and saving an explicit opt-out. Existing Settings tests enumerate the
new page. `scripts/test-update.ps1` provides offline parser, version, asset,
digest, URL, and PE architecture checks and is wired into Windows CI.

Native config, localization, and Settings tests, offline PowerShell tests, the
lifecycle smoke matrix, and English/Turkish page captures were pending when this
record was written. Full download, install, and rollback testing was also pending.
