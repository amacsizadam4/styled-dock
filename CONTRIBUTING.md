# Contributing

Thanks for taking a look. This is a learning project made with AI assistance,
and contributions from people with any level of experience are welcome.

## Ideas and bugs

Open an issue for suggestions, feature requests, questions, or bugs. For a bug,
include what you expected, what happened, your Windows version, and your display
setup (monitor count and scaling). Screenshots help when the problem is visual.
Remove private information before sharing logs, screenshots, or configuration.

For a bigger change, start with an issue so we can discuss the approach. Small
fixes can go straight into a pull request. Explain what changed and how you
checked it. AI-assisted contributions are welcome; please understand the code
you're submitting and mention AI use in the pull request.

## Working on the code

See [the architecture guide](docs/architecture.md) for the folder map. Build on
Windows with Rust's MSVC toolchain and the Windows SDK:

```powershell
cargo fmt --check
cargo build --locked
cargo test --locked window::tests -- --test-threads=1
cargo test --locked config::tests -- --test-threads=1
```

The full suite is `cargo test --locked -- --test-threads=1`. Some tests create
native windows, temporarily change AppBar reservations, or exercise the tray
bridge. Run those in a disposable Windows desktop session with the dock stopped.
They can fail depending on Explorer and desktop state; don't hide failures or
mark them passing without checking. Two visual helpers are ignored by default.

The native end-to-end diagnostic and current Windows/display results are in
[desktop-validation.md](docs/desktop-validation.md). It covers isolated configs,
duplicate launch, fullscreen, normal/forced shutdown, and taskbar restoration.
Physical sleep, Explorer restart, and display changes remain separate checks.

For UI changes, also check the actual app: click, drag, use the keyboard, and try
light/dark appearance and different display scaling. A successful build alone
doesn't establish that the Windows integration works.

Keep pull requests focused. Avoid committing `config.toml`, downloaded files,
build output, caches, or credentials. This repository intentionally keeps its
tooling and documentation simple.
