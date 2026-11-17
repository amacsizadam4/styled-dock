# Localization

English (`en`) and Turkish (`tr`) are the first supported languages. Additional
languages are welcome; the language selector is generated from the registry in
`src/i18n.rs`, rather than a fixed two-language control.

## Add a language

1. Copy `resources/locales/en.toml` to a new language file, such as `de.toml`.
2. Translate the values, keeping the English message keys unchanged. Preserve
   named placeholders such as `{name}`, `{count}`, and `{error}`. Values can reorder
   placeholders. Preserve `\n` and intentional whitespace. `&` marks a native
   keyboard mnemonic; choose a sensible letter without collisions on the page.
3. Register a `Language` entry in `LANGUAGES` in `src/i18n.rs`: language code,
   native display name, Windows formatting locale, and embedded resource path.
4. Run `cargo fmt --check` and `cargo test --locked -- --test-threads=1` on Windows.
   Translation tests check key coverage, empty values, and placeholder parity.
5. Check every Settings page, setup, picker, dock menu, Downloads, status panel,
   light/dark theme, keyboard navigation, and scaling for clipping or ambiguity.
   A right-to-left language will also need an explicit layout review.

## Use messages in code

Use `crate::i18n::text("English message")` for static app text and
`crate::tr!("Could not open {name}.", name = name)` for named values. Register new
messages in every resource. Values are substituted once so braces within an app
name or Windows error remain intact. Singular/plural window labels currently use
separate messages and a count branch; add appropriate plural rules when a new
language requires more forms.

Native dialog captions are captured from their English resource before translation.
Only static and button captions are translated; editable values and app lists
remain user-owned. Shared popup callers translate app-owned labels explicitly,
preserving app/window names, file names, device names, network names, power-plan
names, and other text supplied by Windows or other apps.

`[general].language` persists `auto` or a language code. Auto resolves the Windows
user locale. Unsupported codes/locales fall back to English; a missing message
falls back to its English resource and then its English key. Date/time formatting
uses the selected locale. Setup completion is stored as
`[general].setup_completed`; it is written only when settings save successfully.

Language changes preview in Settings and apply across the app after restart.
Cancel restores the original locale without writing the configuration or changing
Windows startup. Startup availability is read from the actual per-user shortcut,
so it stays in sync with the existing dock menu action.
