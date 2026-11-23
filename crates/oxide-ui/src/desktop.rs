//! The two actions TSX cannot take itself: writing a file the user picked and
//! starting the local compose stack. Both are OS calls, so they stay in Rust.

use nsh_core::file_dialog::SaveDialog;
use nsh_core::prelude::*;

/// Desktop actions the workbench offers from the cross-cutting panel.
#[module]
#[derive(Debug)]
pub struct Desktop;

#[module]
impl Desktop {
    /// Shows the system's save dialog and writes `contents` there; resolves
    /// with the path written, or rejects with why it did not.
    pub async fn save_json(suggested_name: String, contents: String) -> Result<String, String> {
        let dialog = SaveDialog::new()
            .file_name(suggested_name)
            .title("导出 oxide-fab 演示数据")
            .button_label("导出");
        match pick_save_path(dialog).await {
            Ok(Some(path)) => std::fs::write(&path, contents)
                .map(|()| path.display().to_string())
                .map_err(|error| format!("写入失败：{error}")),
            Ok(None) => Err("已取消".to_owned()),
            Err(error) => Err(format!("保存对话框不可用：{error}")),
        }
    }

    /// Runs `docker compose up -d --wait` in `root`; resolves with the status
    /// line, or rejects with the last line compose wrote.
    ///
    /// Compose takes a while and blocks its process, so it runs on a blocking
    /// thread instead of the engine thread.
    pub async fn start_backend(root: String) -> Result<String, String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            match blocking(move || start_compose(root)).await {
                Ok(result) => result,
                Err(error) => Err(format!("启动任务失败：{error}")),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            Err(format!("网页版不能启动本地后端：{root}"))
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn start_compose(root: String) -> Result<String, String> {
    let output = std::process::Command::new("docker")
        .args(["compose", "up", "-d", "--wait"])
        .current_dir(root)
        .output()
        .map_err(|error| format!("无法运行 docker：{error}"))?;
    if output.status.success() {
        Ok("后端已就绪".to_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last = stderr
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("compose 未给出原因")
            .trim();
        Err(format!("compose 失败：{last}"))
    }
}

#[cfg(target_arch = "wasm32")]
fn start_compose(_root: String) -> Result<String, String> {
    Err("网页版不能启动本地后端".to_owned())
}
