//! GitHub updates run outside the UI process so replacement can wait for shutdown.
use std::os::windows::process::CommandExt;
use std::{
    path::PathBuf,
    process::{Child, Command},
    sync::Mutex,
};
use windows::Win32::Foundation::E_FAIL;
use windows::core::{Error, Result};
static WORKER: Mutex<Option<Child>> = Mutex::new(None);
fn root() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("StyledDock/Updates")
}
pub fn start(install: bool) -> Result<()> {
    // Never replace a locally compiled development executable.
    if cfg!(debug_assertions) {
        return Err(Error::new(
            E_FAIL,
            crate::i18n::text("Updates are disabled in development builds."),
        ));
    }
    let mut worker = WORKER.lock().unwrap();
    if let Some(child) = worker.as_mut() {
        if child.try_wait().map_err(Error::from)?.is_none() {
            return Ok(());
        }
    }
    let dir = root();
    std::fs::create_dir_all(&dir).map_err(Error::from)?;
    let script = dir.join("update.ps1");
    std::fs::write(&script, include_str!("../../scripts/update.ps1")).map_err(Error::from)?;
    let mut command = Command::new("powershell.exe");
    command
        .creation_flags(0x08000000)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .arg("-CurrentVersion")
        .arg(env!("CARGO_PKG_VERSION"))
        .arg("-Executable")
        .arg(std::env::current_exe().map_err(Error::from)?)
        .arg("-WorkingDirectory")
        .arg(std::env::current_dir().map_err(Error::from)?)
        .arg("-Root")
        .arg(dir);
    if install {
        command.arg("-Install");
    }
    *worker = Some(command.spawn().map_err(Error::from)?);
    Ok(())
}
pub fn status() -> String {
    let text = std::fs::read_to_string(root().join("status.txt"))
        .unwrap_or_else(|_| "Ready to check for updates.".into());
    // Status identifiers are translated here, rather than in the helper process.
    crate::i18n::text(text.trim_start_matches('\u{feff}').trim()).to_string()
}
pub fn automatic(enabled: bool) {
    if enabled && !cfg!(debug_assertions) {
        // At most one automatic attempt per day; manual actions always bypass this.
        let stamp = root().join("last-check");
        if std::fs::metadata(&stamp)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age.as_secs() < 86400)
        {
            return;
        }
        if start(true).is_ok() {
            let _ = std::fs::write(stamp, []);
        }
    }
}

/// A replacement is healthy only after native windows and services initialized.
pub fn ready() {
    if let Some(path) = std::env::var_os("STYLED_DOCK_UPDATE_READY") {
        let path = PathBuf::from(path);
        if path.parent() == Some(root().as_path()) {
            let _ = std::fs::write(path, b"ready");
        }
    }
}
