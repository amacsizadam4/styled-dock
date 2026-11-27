# Styled Dock

A macOS-inspired dock for Windows. **v1.4.0**

Styled Dock combines an app dock with a small status bar for sound, Wi-Fi,
battery, time, and other Windows controls.

Download the [latest Windows executable or portable ZIP](https://github.com/amacsizadam4/styled-dock/releases/latest).

## About this project

I'm not a professional developer. This project was made using **AI generation
and AI-assisted development**, with ideas, feedback, and testing along the way.
I'm sharing it so other people can try it, understand how it works, and help
improve it. Some parts are experimental, especially the Windows tray integration.

Have a suggestion, found a bug, or want a new feature? **Open an issue.** Pull
requests, explanations, and small improvements are welcome too. You don't need
to be an expert to contribute.

## What it does

- Pin, launch, reorder, and switch between apps.
- Start and Search on the far left by default; drag them wherever you prefer.
- Recent Downloads with file dragging, Windows right-click actions, and Show in folder.
- Sound, network, battery, clock, and quick settings in the top bar.
- Light/dark appearance, hover zoom, and optional auto-hide.
- First-time setup, English/Turkish UI, and optional Windows startup.
- Optional Windows taskbar hiding, with quit and recovery controls.

## Build and run

You'll need Windows, [Rust](https://www.rust-lang.org/tools/install), and the
Visual Studio C++ Build Tools with the Windows SDK. Use Rust's MSVC toolchain.

```powershell
git clone https://github.com/amacsizadam4/styled-dock.git
cd styled-dock
cargo build --release --locked
.\target\release\styled-dock.exe
```

First launch opens setup. Choose your language and startup preferences, review
the other pages, and click **Finish setup**. Settings and setup completion save
locally to `%LOCALAPPDATA%\StyledDock\config.toml` on new installations. Existing
`config.toml` files in the working directory or beside the executable keep priority.
Right-click the dock to open Settings; **General** contains language and Windows
startup controls. Most changes use **Save and restart**.

Windows taskbar hiding is off by default. If you enable it and need to recover:

```powershell
.\target\release\styled-dock.exe --restore-taskbar
```

## Read more

- [Using the dock and Downloads](docs/usage.md)
- [How the code works and where to find things](docs/architecture.md)
- [Current status, known limitations, and roadmap](docs/roadmap.md)
- [Contributing](CONTRIBUTING.md)

This is a personal Windows project, not an official Microsoft or Apple product.
v1.0 is the first public version; it does not yet cover every Windows taskbar
feature. Real hardware and Windows-version coverage are limited. See the roadmap
for known gaps and test limitations.

Released under the [MIT license](LICENSE).
