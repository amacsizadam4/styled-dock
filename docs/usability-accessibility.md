# Everyday usability and accessibility — phase 3

The phase-3 software changes are included in v1.2.0. Hardware, assistive
technology, and the complete visual matrix remain separate verification work.
The remaining phase-2 manual checks are deferred, as requested.

## Mobile hotspot

Quick Settings and the Wi-Fi panel now contain a Mobile hotspot toggle, status,
and a link to **Windows Mobile hotspot settings**. Both read the same cached
state. Queries run on a WinRT worker; changes use the shell worker and an async
operation deadline. The UI disables the toggle while changing or when Windows
reports that control is unavailable, with a reason and a settings link.

The cached state refreshes at startup, resume, and when opening either panel.
Custom panels refresh when a new result arrives. Short/large-scale displays and
high contrast use the native menu fallback; reopen a native fallback menu to
refresh its displayed snapshot. Names and credentials are configured in Windows,
so Styled Dock does not store a hotspot password.

**Not yet verified:** actual enable/disable on compatible hardware and policy
configurations. No test toggled the host's networking. Availability depends on
Windows, its capability checks, the active connection, and the network adapter.

## Keyboard and accessibility

| Shortcut/action | Behavior |
| --- | --- |
| Ctrl+Alt+F | Focus the first dock item and reveal the dock |
| Ctrl+Alt+B | Focus the first status-bar control, when enabled |
| Arrows, Tab/Shift+Tab, Home/End | Navigate the focused dock or status-bar controls |
| Enter or Space | Activate the focused item |
| Menu key or Shift+F10 | Open the focused item's menu; mirrored tray items forward the app's context action |
| Escape | Return to the previously active window when available |
| Ctrl+Alt+D | Open the dock app list |
| Ctrl+Alt+S | Open Quick Settings |

Hotkeys already registered by another app are left alone. App menus include
titled per-window activation and **Close [window title]** commands, in addition
to the existing whole-app commands.

Dock items and drawn status/tray controls have real native button providers for
UI Automation/MSAA. Their names and bounds follow the drawn items. Reordering
keeps a provider tied to its app/control key, not a reused list index. Native
Invoke, keyboard actions, and forwarded pointer gestures use the normal app
commands. The dock draws a keyboard focus outline; status controls highlight the
focused item. Popup focus returns to the prior control when it is still valid.

An independent MTA UI Automation client test checks names, control type, tree
order, bounds, pointer lookup, and Invoke. Keyboard navigation/context routing is
also checked. This establishes provider behavior, not full Narrator/NVDA
compatibility; screen-reader workflows and mixed-DPI navigation remain pending.

## Settings previews and recovery

Size, icon geometry, hover zoom, motion, and both opacity controls preview on the
running dock as valid values change. Theme previews continue to work. Invalid
partial input is kept in the editor without applying it to the dock. On Save,
invalid input selects its page and focuses the relevant field.

**Cancel or closing Settings restores the original appearance**, including after
Restore defaults or Save a copy. Previews do not write the configuration. Most
saved structural changes continue to use **Save and restart**. Standalone
`--settings` has no running dock to preview.

The config path is a keyboard-accessible link: it selects the active file in
Explorer, or opens an existing parent folder before the first save. Link colors
follow the Settings theme. **Save a copy…** exports the edited draft to a chosen
TOML file, preserving the meaning of relative pin targets even in another folder.
It does not change the active config or startup shortcut. It can recover a draft
when a concurrent edit makes the normal save fail. Existing destinations require
the native overwrite confirmation; the active file cannot be selected as its own
recovery copy. Save failures leave Settings open and keep the draft.

## Notification icon and menus

Styled Dock has its own notification icon independently of optional mirroring.
Select it to open Settings, or open its menu for **Settings**, **Restore Windows
taskbar and quit**, and **Quit**. It re-registers after Explorer's TaskbarCreated
notification and removes itself on normal shutdown. Windows decides whether it
appears in the notification-area overflow. When the Windows taskbar is hidden,
the dock menu and keyboard shortcuts remain available.

Menus measure label widths, remain anchored within the monitor work area, and
offer full label/subtitle tooltips. Panels too tall for the work area fall back
to native scrollable menus; high contrast also uses native menus. Native captions
retain card subtitles/status. In that fallback, the embedded volume slider is
unavailable; use the Sound settings action for the Windows volume controls.

## Validation record — 2026-11-19

- Full serial suite: **46 passed, zero failed, two ignored visual helpers**.
  Both final consecutive runs passed.
- The release build and formatting checks passed. All seven isolated-config
  desktop smoke scenarios passed at 125% scale on one display, covering
  floating/reserved/auto-hide placement, normal/forced exit, and top-bar-off.
  This replay kept taskbar hiding and the fullscreen probe disabled; it does not
  replace the deferred desktop lifecycle matrix. Five-second idle samples were
  0.00–1.25% of one core with 57.5–72.2 MiB working sets.
- Regression coverage includes live-preview publication and Cancel rollback,
  field/page selection, recovery copies after concurrent edits, relative pin
  targets in both TOML representations, native UIA/keyboard behavior, notification
  registration/select/delete, oversized-panel fallback, and translation parity.
- The panel artwork helper was run explicitly: **one passed**. Shared panel
  components were rendered in English/Turkish, light/dark, at 96/144/192 DPI
  (100/150/200%). English dark and Turkish light renders were inspected. This is
  component artwork, not screenshots of every native Settings page at those DPIs.
- The actual English Settings window and config hyperlink were inspected at the
  host's 125% scale. Further computer-use input was unavailable because the helper
  reported another active request. The preview process was closed without saving.
- A full suite attempt exposed an occluded point-lookup fixture; that fixture now
  uses a topmost test window. Another run observed an unrelated app changing
  foreground; the status-bar test now checks that the bar/its children did not
  become foreground, instead of requiring an unrelated window to stay active.

The remaining checks are actual hotspot changes, Narrator/NVDA end-to-end use,
pointer/reorder gestures against the native accessibility controls, full
high-contrast behavior on light/dark wallpapers, and every dock/menu/Settings
surface at 100/150/200% and mixed DPI. Phase 3 is not yet declared fully verified.

## Reproduce

```powershell
cargo fmt --check
cargo test --locked -- --test-threads=1
cargo test --locked render_panel_artwork -- --ignored --test-threads=1
cargo build --release --locked
```

Run native desktop tests with the normal dock stopped. Panel BMP artwork is
written under `.test-tmp/control-panel-*.bmp`. See
[desktop-validation.md](desktop-validation.md) for lifecycle diagnostics and the
deferred phase-2 matrix.

API references:
[Windows tethering](https://learn.microsoft.com/en-us/uwp/api/windows.networking.networkoperators.networkoperatortetheringmanager),
[UI Automation providers](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-serversideprovider),
[notification icons](https://learn.microsoft.com/en-us/windows/win32/shell/notification-area),
and [SysLink item colors](https://learn.microsoft.com/en-us/windows/win32/api/commctrl/ns-commctrl-litem).
