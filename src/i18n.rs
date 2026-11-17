//! App-owned text only: never translate app/window names or shell-provided text.
use std::collections::BTreeMap;
use std::sync::{
    LazyLock,
    atomic::{AtomicUsize, Ordering},
};

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::HSTRING;

pub struct Language {
    pub code: &'static str,
    pub name: &'static str,
    locale: &'static str,
    resource: &'static str,
}

// Add a resource and a row here to register a language throughout the app.
pub const LANGUAGES: &[Language] = &[
    Language {
        code: "en",
        name: "English",
        locale: "en-US",
        resource: include_str!("../resources/locales/en.toml"),
    },
    Language {
        code: "tr",
        name: "Türkçe",
        locale: "tr-TR",
        resource: include_str!("../resources/locales/tr.toml"),
    },
];
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static MESSAGES: LazyLock<Vec<BTreeMap<String, String>>> = LazyLock::new(|| {
    LANGUAGES
        .iter()
        .map(|language| {
            toml::from_str(language.resource)
                .expect("bundled translation resource must be valid TOML")
        })
        .collect()
});

fn resolve(choice: &str, windows_language: &str) -> usize {
    let choice = if choice == "auto" || choice.is_empty() {
        windows_language
    } else {
        choice
    };
    let base = choice.split(['-', '_']).next().unwrap_or(choice);
    LANGUAGES
        .iter()
        .position(|language| language.code.eq_ignore_ascii_case(base))
        .unwrap_or(0)
}

pub fn configure(choice: &str) {
    let mut locale = [0; 85];
    let length =
        unsafe { LCIDToLocaleName(u32::from(GetUserDefaultUILanguage()), Some(&mut locale), 0) };
    let windows_language = String::from_utf16_lossy(&locale[..length.saturating_sub(1) as usize]);
    ACTIVE.store(resolve(choice, &windows_language), Ordering::Relaxed);
}

fn lookup(language: usize, key: &str) -> &str {
    MESSAGES
        .get(language)
        .and_then(|messages| messages.get(key))
        .or_else(|| MESSAGES[0].get(key))
        .map_or(key, String::as_str)
}

pub fn text(key: &str) -> &str {
    lookup(ACTIVE.load(Ordering::Relaxed), key)
}

pub fn message(key: &str, arguments: &[(&str, String)]) -> String {
    interpolate(text(key), arguments)
}

fn interpolate(template: &str, arguments: &[(&str, String)]) -> String {
    // Replace tokens in a single pass so values containing braces are preserved.
    let mut output = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        output.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}').map(|end| start + end) else {
            rest = &rest[start..];
            break;
        };
        let name = &rest[start + 1..end];
        if let Some((_, value)) = arguments.iter().find(|(key, _)| *key == name) {
            output.push_str(value);
        } else {
            output.push_str(&rest[start..=end]);
        }
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    output
}

#[macro_export]
macro_rules! tr {
    ($key:literal $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::i18n::message($key, &[$((stringify!($name), ($value).to_string())),*])
    };
}

pub fn locale() -> HSTRING {
    HSTRING::from(LANGUAGES[ACTIVE.load(Ordering::Relaxed)].locale)
}

pub fn translations(key: &str) -> Vec<HSTRING> {
    (0..LANGUAGES.len())
        .map(|index| HSTRING::from(lookup(index, key)))
        .collect()
}

pub fn captions(hwnd: HWND) -> Vec<(HWND, String)> {
    unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
        let mut class = [0; 64];
        let length = unsafe { GetClassNameW(hwnd, &mut class) };
        let class = String::from_utf16_lossy(&class[..length as usize]);
        if class == "Static" || class == "Button" {
            let entries = unsafe { &mut *(data.0 as *mut Vec<(HWND, String)>) };
            entries.push((hwnd, window_text(hwnd)));
        }
        true.into()
    }
    let mut entries = vec![(hwnd, window_text(hwnd))];
    unsafe {
        let _ = EnumChildWindows(
            Some(hwnd),
            Some(collect),
            LPARAM((&raw mut entries) as isize),
        );
    }
    entries
}

fn window_text(hwnd: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    let mut buffer = vec![0; length.max(0) as usize + 1];
    let length = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..length as usize])
}

pub fn translate_captions(captions: &[(HWND, String)]) {
    for (hwnd, key) in captions {
        unsafe {
            let _ = SetWindowTextW(*hwnd, &HSTRING::from(text(key)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_auto_overrides_and_unsupported_languages() {
        assert_eq!(resolve("auto", "tr-TR"), 1);
        assert_eq!(resolve("en", "tr-TR"), 0);
        assert_eq!(resolve("tr", "en-US"), 1);
        assert_eq!(resolve("auto", "fr-FR"), 0);
        assert_eq!(resolve("unknown", "tr-TR"), 0);
        assert_eq!(lookup(99, "Cancel"), "Cancel");
        assert_eq!(lookup(1, "missing key"), "missing key");
    }

    #[test]
    fn translations_cover_english_and_preserve_placeholders() {
        fn tokens(value: &str) -> Vec<&str> {
            let mut tokens: Vec<_> = value
                .split('{')
                .skip(1)
                .filter_map(|part| part.split_once('}').map(|(token, _)| token))
                .collect();
            tokens.sort();
            tokens
        }
        for messages in MESSAGES.iter() {
            assert_eq!(messages.len(), MESSAGES[0].len());
            for (key, english) in &MESSAGES[0] {
                let translation = messages.get(key).unwrap_or_else(|| panic!("missing {key}"));
                assert!(!translation.is_empty(), "empty translation: {key}");
                assert_eq!(tokens(english), tokens(translation), "placeholders: {key}");
            }
        }
        assert_eq!(lookup(1, "Cancel"), "İptal");
        assert_eq!(
            interpolate(lookup(1, "Battery {percent}%"), &[("percent", "73".into())]),
            "Pil %73"
        );
        assert_eq!(
            interpolate("{name} / {name}", &[("name", "{percent}".into())]),
            "{percent} / {percent}"
        );
        assert_eq!(interpolate("Unclosed {token", &[]), "Unclosed {token");
    }
}
