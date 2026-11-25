//! Automatic read-only HUD, launched once per runtime process.
use std::sync::Once;
use std::process::{Command, Stdio};
use std::os::windows::process::CommandExt;

pub(crate) fn start() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if std::env::var_os("UMBRA_HUD").is_some_and(|v| v == "0") {
            return;
        }
        let mut cmd = if let Some(python) = std::env::var_os("UMBRA_HUD_PYTHON") {
            Command::new(python)
        } else {
            let mut cmd = Command::new("py.exe");
            cmd.arg("-3");
            cmd
        };
        cmd.arg("-I")
            .arg("-c").arg(include_str!("umbra_hud_autostart.py"))
            .arg("--pid").arg(std::process::id().to_string())
            .arg("--exit-with-parent")
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .creation_flags(0x08000000); // CREATE_NO_WINDOW
        if let Some(corner) = std::env::var_os("UMBRA_HUD_CORNER") {
            if corner == "left" || corner == "right" { cmd.arg("--corner").arg(corner); }
        }
        if let Some(log) = std::env::var_os("UMBRA_HUD_LOG") {
            cmd.arg("--log").arg(log);
        }
        match cmd.spawn() {
            Ok(child) => tracing::info!(pid = child.id(), "UMBRA HUD v0.2 auto-started"),
            Err(error) => tracing::warn!(%error, "UMBRA HUD could not start; runtime continues"),
        }
    });
}
