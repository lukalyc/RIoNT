//! Product-side background operations. These are RIONT features, not NT4
//! protocol: the `riont-nt4` engine crate stays free of them.

use crate::app::ToastKind;
use tokio::sync::mpsc::UnboundedSender;

/// Results from background operations, delivered to the UI as toasts.
pub type ToastTx = UnboundedSender<(ToastKind, String)>;

/// Fire-and-forget SSH restart of the robot user code. Runs entirely in a
/// background task; the outcome lands back in the UI as a toast. Never
/// touches NetworkTables — the command comes straight from config.json
/// (`system.ssh_user` + `system.restart_cmd`).
pub fn spawn_restart(toasts: ToastTx, host: String, user: String, cmd: String) {
    tokio::spawn(async move {
        let started = tokio::time::Instant::now();
        let output = tokio::process::Command::new("ssh")
            // tokio's output() leaves stdin INHERITED (unlike std), which
            // would hand the TUI's keystroke pipe to ssh and hang it.
            .stdin(std::process::Stdio::null())
            .arg("-o")
            .arg("ConnectTimeout=5")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("StrictHostKeyChecking=no")
            .arg(format!("{}@{}", user, host))
            .arg(&cmd)
            .output()
            .await;
        let elapsed = started.elapsed().as_secs_f32();
        let result = match output {
            Ok(out) if out.status.success() => (
                ToastKind::Success,
                format!("robot code restarted in {:.2}s", elapsed),
            ),
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let detail = stderr
                    .lines()
                    .last()
                    .unwrap_or("ssh failed")
                    .trim()
                    .to_string();
                (ToastKind::Error, format!("restart failed: {}", detail))
            }
            Err(e) => (
                ToastKind::Error,
                format!("restart: ssh unavailable ({})", e),
            ),
        };
        toasts.send(result).ok();
    });
}
