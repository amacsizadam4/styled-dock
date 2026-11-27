# Styled Dock development

Develop from a local Windows checkout with Rust x64 MSVC and the Windows SDK.
Run Git and build from that same checkout so builds include uncommitted edits.
Use the logged-in Windows desktop for Launch, Smoke, and tests that create
native windows or interact with Explorer.

Quit any running Styled Dock instance before rebuilding or testing. From the
repository root in regular PowerShell, pick the smallest relevant check.

```powershell
cargo fmt --check
.\scripts\dev.ps1 Test config::tests
.\scripts\dev.ps1 Launch
.\scripts\dev.ps1 Smoke -Fullscreen
```

The script keeps Cargo output and complete logs under
`%LOCALAPPDATA%\StyledDockDev`. Keep tool output to the summary and relevant
failure lines. Run desktop checks serially on the interactive Windows desktop.

Use focused tests for config and localization, and add a targeted desktop
check for changes to visuals, keyboard input, or focus. Capture the affected
region at native scale when appearance matters. Capture the full desktop when
checking monitor placement, clipping, or the taskbar relationship. Take another
capture when it answers a new question about a changed build or state.

Use the smoke matrix for placement, fullscreen, taskbar, and lifecycle changes.
See `docs/architecture.md` for entry points and `docs/desktop-validation.md` for
smoke options, taskbar recovery, and hardware checks.
