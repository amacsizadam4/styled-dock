use std::collections::HashSet;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, value};
use windows::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::core::{Error, HSTRING, Result};

const DEFAULTS: &str = include_str!("../config.example.toml");
const APPS_FOLDER: &str = "shell:AppsFolder\\";
pub const MAX_PINS: usize = 32;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub general: General,
    pub pinned: Vec<Pin>,
    #[serde(default)]
    pub dock: Dock,
    #[serde(default)]
    pub placement: Placement,
    #[serde(default)]
    pub top_bar: TopBar,
    #[serde(default)]
    pub appearance: Appearance,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct General {
    pub setup_completed: bool,
    #[serde(default = "automatic_language")]
    pub language: String,
}

impl Default for General {
    fn default() -> Self {
        Self {
            setup_completed: false,
            language: automatic_language(),
        }
    }
}

fn automatic_language() -> String {
    "auto".into()
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Dock {
    pub show_start: bool,
    pub show_search: bool,
    pub show_downloads: bool,
    pub start_position: usize,
    pub search_position: usize,
    pub search_first: bool,
}

impl Default for Dock {
    fn default() -> Self {
        Self {
            show_start: true,
            show_search: true,
            show_downloads: true,
            start_position: 0,
            search_position: 0,
            search_first: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Appearance {
    pub theme: crate::theme::ThemeMode,
    pub size_percent: u32,
    pub icon_size: u32,
    pub icon_spacing: u32,
    pub edge_padding: u32,
    pub label_delay_ms: u32,
    pub zoom_percent: u32,
    pub animation_speed_percent: u32,
    pub launch_bounce: bool,
    pub auto_hide_delay_ms: u32,
    pub dock_opacity_percent: u32,
    pub top_bar_opacity_percent: u32,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: crate::theme::ThemeMode::Auto,
            size_percent: 100,
            icon_size: 48,
            icon_spacing: 12,
            edge_padding: 16,
            label_delay_ms: 1000,
            zoom_percent: 68,
            animation_speed_percent: 100,
            launch_bounce: true,
            auto_hide_delay_ms: 350,
            dock_opacity_percent: 78,
            top_bar_opacity_percent: 36,
        }
    }
}

impl Appearance {
    pub fn validate(self) -> std::result::Result<(), &'static str> {
        if !(60..=160).contains(&self.size_percent) {
            return Err(crate::i18n::text("Dock size must be between 60% and 160%."));
        }
        if !(24..=96).contains(&self.icon_size) {
            return Err(crate::i18n::text("Icon size must be between 24 and 96 px."));
        }
        if self.icon_spacing > 32 {
            return Err(crate::i18n::text(
                "Icon spacing must be between 0 and 32 px.",
            ));
        }
        if !(4..=40).contains(&self.edge_padding) {
            return Err(crate::i18n::text(
                "Edge padding must be between 4 and 40 px.",
            ));
        }
        if self.label_delay_ms > 3000 {
            return Err(crate::i18n::text(
                "App label delay must be between 0 and 3000 ms.",
            ));
        }
        if self.zoom_percent > 100 {
            return Err(crate::i18n::text("Hover zoom must be between 0% and 100%."));
        }
        if self.animation_speed_percent != 0 && !(25..=200).contains(&self.animation_speed_percent)
        {
            return Err(crate::i18n::text(
                "Animation speed must be 0% for instant transitions or between 25% and 200%.",
            ));
        }
        if !(100..=3000).contains(&self.auto_hide_delay_ms) {
            return Err(crate::i18n::text(
                "Auto-hide delay must be between 100 and 3000 ms.",
            ));
        }
        if self.dock_opacity_percent > 100 || self.top_bar_opacity_percent > 100 {
            return Err(crate::i18n::text(
                "Background opacity must be between 0% and 100%.",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TopBar {
    pub enabled: bool,
    pub show_tray: bool,
}

impl Default for TopBar {
    fn default() -> Self {
        Self {
            enabled: true,
            show_tray: true,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Placement {
    pub monitor: String,
    pub reserve_space: bool,
    pub auto_hide: bool,
    pub hide_on_fullscreen: bool,
    pub hide_taskbar: bool,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            monitor: "primary".into(),
            reserve_space: false,
            auto_hide: false,
            hide_on_fullscreen: true,
            hide_taskbar: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    pub name: String,
    pub target: String,
}

pub fn load() -> Result<Config> {
    let source = Source::open()?;
    if let Ok(draft) = source.draft() {
        crate::i18n::configure(&draft.general.language);
    }
    match parse(&source.text, &source.path) {
        Ok(config) => Ok(config),
        Err(error) => {
            use windows::Win32::UI::WindowsAndMessaging::*;
            let answer = unsafe {
                MessageBoxW(
                    None,
                    &HSTRING::from(crate::tr!(
                        "The configuration could not be loaded.\n\n{error}\n\nBack up this file and reset to defaults?",
                        error = error
                    )),
                    &HSTRING::from(crate::i18n::text("Styled Dock configuration")),
                    MB_YESNO | MB_ICONWARNING,
                )
            };
            if answer != IDYES {
                return Err(error);
            }
            let backup = source.path.with_extension(format!(
                "toml.backup-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ));
            fs::copy(&source.path, &backup).map_err(io_error)?;
            replace_config(&source.path, DEFAULTS)?;
            parse(DEFAULTS, &source.path)
        }
    }
}

pub struct Source {
    pub path: PathBuf,
    text: String,
    existed: bool,
}

impl Source {
    pub fn open() -> Result<Self> {
        let cwd = std::env::current_dir().map_err(io_error)?;
        let executable = std::env::current_exe().map_err(io_error)?;
        let candidates = [
            cwd.join("config.toml"),
            executable.with_file_name("config.toml"),
            user_directory()?.join("config.toml"),
        ];
        for path in &candidates {
            match fs::read_to_string(path) {
                Ok(text) => {
                    return Ok(Self {
                        path: path.clone(),
                        text,
                        existed: true,
                    });
                }
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => return Err(invalid(path, error)),
            }
        }
        Ok(Self {
            path: candidates[2].clone(),
            text: DEFAULTS.into(),
            existed: false,
        })
    }

    pub fn draft(&self) -> Result<Config> {
        decode(&self.text, &self.path)
    }

    pub fn defaults(&self) -> Result<Config> {
        decode(DEFAULTS, &self.path)
    }

    pub fn save(&self, config: &Config) -> Result<()> {
        self.replace(&self.contents(config)?)
    }

    /// Keep pin targets valid when a recovery copy is saved in another folder.
    pub fn save_copy(&self, path: &Path, config: &Config) -> Result<()> {
        if fs::canonicalize(path)
            .ok()
            .zip(fs::canonicalize(&self.path).ok())
            .is_some_and(|(a, b)| a == b)
            || path == self.path
        {
            return Err(invalid(
                path,
                crate::i18n::text("Choose a different file for the copy."),
            ));
        }
        let contents = self.contents(config)?;
        let resolved = parse(&contents, &self.path)?;
        let mut document: DocumentMut = contents.parse().map_err(|error| invalid(path, error))?;
        if !resolved.pinned.is_empty() {
            let mut tables = ArrayOfTables::new();
            for pin in &resolved.pinned {
                tables.push(pin_table(pin));
            }
            document["pinned"] = Item::ArrayOfTables(tables);
        }
        let contents = document.to_string();
        parse(&contents, path)?;
        let (text, existed) = match fs::read_to_string(path) {
            Ok(text) => (text, true),
            Err(error) if error.kind() == ErrorKind::NotFound => (String::new(), false),
            Err(error) => return Err(invalid(path, error)),
        };
        Source {
            path: path.into(),
            text,
            existed,
        }
        .replace(&contents)
    }

    fn contents(&self, config: &Config) -> Result<String> {
        let mut document: DocumentMut = self
            .text
            .parse()
            .map_err(|error| invalid(&self.path, error))?;
        if !document.contains_key("general") {
            document["general"] = Item::Table(Table::new());
        }
        put(
            &mut document["general"]["language"],
            value(&config.general.language),
        );
        put(
            &mut document["general"]["setup_completed"],
            value(config.general.setup_completed),
        );
        if !document.contains_key("dock") {
            document["dock"] = Item::Table(Table::new());
        }
        write_shortcut_positions(&mut document, config.dock);
        for (key, enabled) in [
            ("show_start", config.dock.show_start),
            ("show_search", config.dock.show_search),
            ("show_downloads", config.dock.show_downloads),
        ] {
            put(&mut document["dock"][key], value(enabled));
        }
        let placement = &config.placement;
        if !document.contains_key("placement") {
            document["placement"] = Item::Table(Table::new());
        }
        if !document.contains_key("top_bar") {
            document["top_bar"] = Item::Table(Table::new());
        }
        put(
            &mut document["placement"]["monitor"],
            value(&placement.monitor),
        );
        for (key, enabled) in [
            ("reserve_space", placement.reserve_space),
            ("auto_hide", placement.auto_hide),
            ("hide_on_fullscreen", placement.hide_on_fullscreen),
            ("hide_taskbar", placement.hide_taskbar),
        ] {
            put(&mut document["placement"][key], value(enabled));
        }
        put(
            &mut document["top_bar"]["enabled"],
            value(config.top_bar.enabled),
        );
        put(
            &mut document["top_bar"]["show_tray"],
            value(config.top_bar.show_tray),
        );
        if !document.contains_key("appearance") {
            document["appearance"] = Item::Table(Table::new());
        }
        let appearance = config.appearance;
        put(
            &mut document["appearance"]["theme"],
            value(appearance.theme.name()),
        );
        for (key, amount) in [
            ("size_percent", appearance.size_percent),
            ("icon_size", appearance.icon_size),
            ("icon_spacing", appearance.icon_spacing),
            ("edge_padding", appearance.edge_padding),
            ("label_delay_ms", appearance.label_delay_ms),
            ("zoom_percent", appearance.zoom_percent),
            (
                "animation_speed_percent",
                appearance.animation_speed_percent,
            ),
            ("auto_hide_delay_ms", appearance.auto_hide_delay_ms),
            ("dock_opacity_percent", appearance.dock_opacity_percent),
            (
                "top_bar_opacity_percent",
                appearance.top_bar_opacity_percent,
            ),
        ] {
            put(&mut document["appearance"][key], value(i64::from(amount)));
        }
        put(
            &mut document["appearance"]["launch_bounce"],
            value(appearance.launch_bounce),
        );

        if config.pinned != self.draft()?.pinned {
            if config.pinned.is_empty() {
                document["pinned"] = value(Array::new());
            } else {
                let mut tables = ArrayOfTables::new();
                for pin in &config.pinned {
                    tables.push(pin_table(pin));
                }
                document["pinned"] = Item::ArrayOfTables(tables);
            }
        }
        let contents = document.to_string();
        parse(&contents, &self.path)?;
        Ok(contents)
    }

    pub fn save_pins(&self, previous: &[Pin], pins: &[Pin]) -> Result<()> {
        self.save_order(previous, pins, None)
    }

    pub fn save_order(&self, previous: &[Pin], pins: &[Pin], dock: Option<Dock>) -> Result<()> {
        if parse(&self.text, &self.path)?.pinned != previous {
            return Err(invalid(
                &self.path,
                crate::i18n::text("pins changed on disk. Restart Styled Dock and try again."),
            ));
        }
        let mut document: DocumentMut = self
            .text
            .parse()
            .map_err(|error| invalid(&self.path, error))?;
        let draft = self.draft()?;
        if let Some(dock) = dock {
            write_shortcut_positions(&mut document, dock);
        }
        let mut tables = ArrayOfTables::new();
        for pin in pins {
            // Keep the original target spelling, including relative paths and variables.
            let mut table = if let Some(index) = previous.iter().position(|old| old == pin) {
                document["pinned"]
                    .as_array_of_tables()
                    .and_then(|tables| tables.get(index))
                    .cloned()
                    .unwrap_or_else(|| pin_table(&draft.pinned[index]))
            } else {
                pin_table(pin)
            };
            table.set_position(None);
            tables.push(table);
        }
        document["pinned"] = if tables.is_empty() {
            value(Array::new())
        } else {
            Item::ArrayOfTables(tables)
        };
        let contents = document.to_string();
        if parse(&contents, &self.path)?.pinned != pins {
            return Err(invalid(
                &self.path,
                crate::i18n::text("a pin target could not be stored as entered"),
            ));
        }
        self.replace(&contents)
    }

    fn replace(&self, contents: &str) -> Result<()> {
        match fs::read_to_string(&self.path) {
            Ok(current) if self.existed && current == self.text => {}
            Err(error) if !self.existed && error.kind() == ErrorKind::NotFound => {}
            Err(error) if error.kind() != ErrorKind::NotFound => {
                return Err(invalid(&self.path, error));
            }
            _ => {
                return Err(invalid(
                    &self.path,
                    crate::i18n::text(
                        "the file changed while it was being edited. Try again with the current configuration.",
                    ),
                ));
            }
        }
        replace_config(&self.path, contents)
    }
}

fn write_shortcut_positions(document: &mut DocumentMut, dock: Dock) {
    if !document.contains_key("dock") {
        document["dock"] = Item::Table(Table::new());
    }
    put(
        &mut document["dock"]["start_position"],
        value(dock.start_position.min(i64::MAX as usize) as i64),
    );
    put(
        &mut document["dock"]["search_position"],
        value(dock.search_position.min(i64::MAX as usize) as i64),
    );
    put(
        &mut document["dock"]["search_first"],
        value(dock.search_first),
    );
}

fn pin_table(pin: &Pin) -> Table {
    let mut table = Table::new();
    table["name"] = value(&pin.name);
    table["target"] = value(&pin.target);
    table
}

fn put(item: &mut Item, mut replacement: Item) {
    if let (Some(previous), Some(next)) = (item.as_value(), replacement.as_value_mut()) {
        if previous
            .as_str()
            .zip(next.as_str())
            .is_some_and(|(a, b)| a == b)
            || previous
                .as_bool()
                .zip(next.as_bool())
                .is_some_and(|(a, b)| a == b)
            || previous
                .as_integer()
                .zip(next.as_integer())
                .is_some_and(|(a, b)| a == b)
        {
            return;
        }
        *next.decor_mut() = previous.decor().clone();
    }
    *item = replacement;
}

pub(crate) fn user_directory() -> Result<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, None)?;
        let result = path.to_string();
        CoTaskMemFree(Some(path.0.cast()));
        Ok(PathBuf::from(result?).join("StyledDock"))
    }
}

fn replace_config(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = path.with_file_name(format!(".config-{}-{nonce}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(io_error)?;
    let written = file
        .write_all(contents.as_bytes())
        .and_then(|()| file.sync_all());
    drop(file);
    let result = written.map_err(io_error).and_then(|()| unsafe {
        MoveFileExW(
            &HSTRING::from(temporary.as_os_str()),
            &HSTRING::from(path.as_os_str()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    });
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn decode(contents: &str, path: &Path) -> Result<Config> {
    toml::from_str(contents).map_err(|error| invalid(path, error))
}

fn parse(contents: &str, path: &Path) -> Result<Config> {
    let mut config = decode(contents, path)?;
    config
        .appearance
        .validate()
        .map_err(|error| invalid(path, error))?;
    if config.pinned.len() > MAX_PINS {
        return Err(invalid(
            path,
            crate::i18n::text("pinned supports at most 32 entries"),
        ));
    }
    if config.placement.reserve_space && config.placement.auto_hide {
        return Err(invalid(
            path,
            crate::i18n::text("reserve_space and auto_hide cannot both be enabled"),
        ));
    }
    if config.placement.monitor.trim().is_empty() {
        return Err(invalid(
            path,
            crate::i18n::text("placement.monitor must name a display, primary, or cursor"),
        ));
    }
    let mut targets = HashSet::new();
    for pin in &mut config.pinned {
        if pin.name.trim().is_empty()
            || pin.target.trim().is_empty()
            || pin.name.contains('\0')
            || pin.target.contains('\0')
        {
            return Err(invalid(
                path,
                crate::i18n::text("each pin needs a nonempty name and target"),
            ));
        }
        let target = expand(&pin.target)?;
        if is_packaged(&target) {
            if target.len() == APPS_FOLDER.len() {
                return Err(invalid(
                    path,
                    crate::i18n::text("shell:AppsFolder needs an application ID"),
                ));
            }
            pin.target = target;
        } else {
            let target = PathBuf::from(target);
            let target = if target.is_absolute() {
                target
            } else {
                path.parent().unwrap_or(Path::new(".")).join(target)
            };
            pin.target = target.to_string_lossy().into_owned();
        }
        if !targets.insert(pin.target.to_lowercase()) {
            return Err(invalid(
                path,
                crate::tr!("duplicate target {target}", target = pin.target),
            ));
        }
    }
    Ok(config)
}

pub fn is_packaged(target: &str) -> bool {
    target
        .get(..APPS_FOLDER.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(APPS_FOLDER))
}

fn expand(value: &str) -> Result<String> {
    let source = HSTRING::from(value);
    let mut length = unsafe { ExpandEnvironmentStringsW(&source, None) };
    loop {
        if length == 0 {
            return Err(Error::from_thread());
        }
        let mut buffer = vec![0; length as usize];
        length = unsafe { ExpandEnvironmentStringsW(&source, Some(&mut buffer)) };
        if length > 0 && length as usize <= buffer.len() {
            return Ok(String::from_utf16_lossy(&buffer[..length as usize - 1]));
        }
    }
}

fn invalid(path: &Path, message: impl std::fmt::Display) -> Error {
    Error::new(
        windows::Win32::Foundation::E_INVALIDARG,
        format!("{}: {message}", path.display()),
    )
}

fn io_error(error: std::io::Error) -> Error {
    Error::new(windows::Win32::Foundation::E_FAIL, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_copy_keeps_draft_and_relative_pin_meaning_without_overwriting_active_file()
    -> Result<()> {
        let root = std::env::current_dir()
            .map_err(io_error)?
            .join(".test-tmp")
            .join(format!("config-copy-{}", std::process::id()));
        fs::create_dir_all(root.join("export")).map_err(io_error)?;
        let path = root.join("config.toml");
        let original = "[[pinned]]\nname = 'Editor'\ntarget = 'editor.exe'\n";
        fs::write(&path, original).map_err(io_error)?;
        let source = Source {
            path: path.clone(),
            text: original.into(),
            existed: true,
        };
        let mut draft = source.draft()?;
        draft.appearance.size_percent = 140;
        draft.general.language = "tr".into();
        // Simulate a concurrent editor: a normal save must fail, but exporting
        // the user's draft must remain possible without losing the external edit.
        fs::write(&path, "pinned = []\n").map_err(io_error)?;
        assert!(source.save(&draft).is_err());
        let exported = root.join("export/settings.toml");
        source.save_copy(&exported, &draft)?;
        let copy = parse(&fs::read_to_string(&exported).map_err(io_error)?, &exported)?;
        assert_eq!(copy.appearance.size_percent, 140);
        assert_eq!(copy.general.language, "tr");
        assert_eq!(
            copy.pinned[0].target,
            root.join("editor.exe").to_string_lossy()
        );
        assert_eq!(
            fs::read_to_string(&path).map_err(io_error)?,
            "pinned = []\n"
        );
        assert!(source.save_copy(&path, &draft).is_err());
        let inline = Source {
            path: path.clone(),
            text: "pinned = [{ name = 'Editor', target = 'editor.exe' }]\n".into(),
            existed: true,
        };
        inline.save_copy(&exported, &inline.draft()?)?;
        let copy = parse(&fs::read_to_string(&exported).map_err(io_error)?, &exported)?;
        assert_eq!(
            copy.pinned[0].target,
            root.join("editor.exe").to_string_lossy()
        );
        fs::remove_file(exported).map_err(io_error)?;
        fs::remove_file(path).map_err(io_error)?;
        fs::remove_dir(root.join("export")).map_err(io_error)?;
        fs::remove_dir(root).map_err(io_error)?;
        Ok(())
    }

    #[test]
    fn appearance_defaults_and_partial_config_are_compatible() -> Result<()> {
        let path = Path::new("config.toml");
        let legacy = parse("pinned = []", path)?;
        assert!(!legacy.general.setup_completed);
        assert_eq!(legacy.general.language, "auto");
        assert_eq!(parse("pinned = []", path)?.dock, Dock::default());
        assert_eq!(
            parse("pinned = []\n[dock]\nshow_downloads = false", path)?.dock,
            Dock {
                show_downloads: false,
                ..Default::default()
            }
        );
        assert_eq!(
            parse("pinned = []", path)?.appearance,
            Appearance::default()
        );
        for (name, mode) in [
            ("auto", crate::theme::ThemeMode::Auto),
            ("light", crate::theme::ThemeMode::Light),
            ("dark", crate::theme::ThemeMode::Dark),
        ] {
            let text = format!("pinned = []\n[appearance]\ntheme = \"{name}\"");
            assert_eq!(parse(&text, path)?.appearance.theme, mode);
        }
        assert!(parse("pinned = []\n[appearance]\ntheme = \"invalid\"", path).is_err());
        let tuned = parse(
            "pinned = []\n[appearance]\nsize_percent = 140\nzoom_percent = 0\nanimation_speed_percent = 0",
            path,
        )?;
        assert_eq!(
            tuned.appearance,
            Appearance {
                size_percent: 140,
                zoom_percent: 0,
                animation_speed_percent: 0,
                ..Appearance::default()
            }
        );
        assert!(parse("pinned = []\n[appearance]\nsize_percent = 0", path).is_err());
        for invalid in [
            "icon_size = 23",
            "icon_size = 97",
            "icon_spacing = 33",
            "edge_padding = 3",
            "edge_padding = 41",
            "label_delay_ms = 3001",
        ] {
            assert!(parse(&format!("pinned = []\n[appearance]\n{invalid}"), path).is_err());
        }
        Ok(())
    }

    #[test]
    fn dock_pin_edits_keep_saved_order_and_original_targets() -> Result<()> {
        let path =
            std::env::temp_dir().join(format!("styled-dock-pin-test-{}.toml", std::process::id()));
        let original = r#"
[top_bar]
enabled = false

[[pinned]]
name = "Editor"
target = 'editor.exe' # Relative to this file

[[pinned]]
name = "Explorer"
target = '%WINDIR%\explorer.exe'
"#;
        fs::write(&path, original).map_err(io_error)?;
        let result = (|| {
            let source = Source {
                path: path.clone(),
                text: original.into(),
                existed: true,
            };
            let previous = parse(original, &path)?.pinned;
            let added = Pin {
                name: "Calculator".into(),
                target: "shell:AppsFolder\\Example!App".into(),
            };
            let updated = vec![previous[1].clone(), added, previous[0].clone()];
            source.save_pins(&previous, &updated)?;
            let saved = fs::read_to_string(&path).map_err(io_error)?;
            let config = parse(&saved, &path)?;
            assert_eq!(config.pinned, updated);
            assert!(!config.top_bar.enabled);
            assert!(saved.contains("'editor.exe' # Relative to this file"));
            assert!(saved.contains(r"'%WINDIR%\explorer.exe'"));
            assert!(source.save_pins(&previous, &[]).is_err());

            let source = Source {
                path: path.clone(),
                text: saved,
                existed: true,
            };
            source.save_pins(&updated, &[])?;
            let saved = fs::read_to_string(&path).map_err(io_error)?;
            assert!(parse(&saved, &path)?.pinned.is_empty());
            let source = Source {
                path: path.clone(),
                text: saved,
                existed: true,
            };
            let mut draft = source.draft()?;
            draft.general.language = "tr".into();
            draft.general.setup_completed = true;
            draft.appearance.icon_size = 72;
            draft.appearance.icon_spacing = 0;
            draft.appearance.edge_padding = 24;
            draft.appearance.label_delay_ms = 1500;
            draft.top_bar.show_tray = false;
            draft.dock = Dock {
                show_start: false,
                show_search: true,
                show_downloads: false,
                start_position: 5,
                search_position: 2,
                search_first: true,
            };
            source.save(&draft)?;
            let saved = parse(&fs::read_to_string(&path).map_err(io_error)?, &path)?;
            assert_eq!(saved.general.language, "tr");
            assert!(saved.general.setup_completed);
            assert_eq!(saved.appearance, draft.appearance);
            assert_eq!(saved.dock, draft.dock);
            assert!(!saved.top_bar.show_tray);
            let text = fs::read_to_string(&path).map_err(io_error)?;
            let source = Source {
                path: path.clone(),
                text,
                existed: true,
            };
            let mut moved = saved.dock;
            moved.start_position = 9;
            moved.search_position = 0;
            source.save_order(&saved.pinned, &saved.pinned, Some(moved))?;
            let restored = parse(&fs::read_to_string(&path).map_err(io_error)?, &path)?;
            assert_eq!(restored.dock, moved);
            assert_eq!(restored.pinned, saved.pinned);
            Ok(())
        })();
        let _ = fs::remove_file(path);
        result
    }
}
