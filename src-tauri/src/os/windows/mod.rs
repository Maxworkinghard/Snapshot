//! Windows 平台层。
//!
//! 与 macOS / Linux 平台层对外提供同名的一组函数（见 lib.rs 里 `mod os` 的说明），
//! 共享代码只经 `os::` 调用，不再到处写 `#[cfg(target_os = …)]`。
//! 实现都用系统原生能力：WGC + Media Foundation 录制、注册表自启。

mod apps;
mod autostart;
mod cursor;
mod icon;
mod recorder;
mod window;

use crate::capabilities::{CapabilityStatus, PlatformCapabilities};
use crate::capture::Area;
use crate::recording::RecordOptions;
use image::RgbaImage;
use std::{fs, path::Path, process::Command};

pub(crate) use apps::{installed_apps, launch_icon_png, launch_target};
pub(crate) use autostart::apply as apply_autostart;
pub(crate) use icon::app_icon_png;
pub(crate) use window::restore_minimized;

/// 本平台支持的全局快捷键动作，顺序即设置页的显示顺序
pub(crate) const SHORTCUT_ACTIONS: &[&str] =
    &["snapshot", "fullscreen", "record", "polish", "palette"];

/// 一次进行中的录制
pub(crate) struct Recording(recorder::ActiveRecording);

impl Recording {
    /// WGC 会话不会自己中途退出，没有「意外结束」可报
    pub(crate) fn exit_reason(&mut self) -> Option<String> {
        None
    }

    pub(crate) fn stop(self) -> Result<(), String> {
        // stop 内部会 Finalize，MP4 的 moov 在这一步才写进去
        let (path, frames) = self.0.stop();
        // 一帧都没有时产物是个播放器打不开的空壳，留着只会让人以为录成功了
        if frames == 0 {
            let _ = fs::remove_file(&path);
            eprintln!(
                "snapshot: recording produced no frames, removed {}",
                path.display()
            );
        }
        Ok(())
    }
}

/// xcap 在 Windows 上的窗口 id 就是 HWND，直接交给 WGC 按句柄采集，
/// 不必像 ffmpeg 那样按标题找窗口。
pub(crate) fn start_recording(
    window_id: u32,
    options: &RecordOptions,
    output: &Path,
) -> Result<(Recording, Option<String>), String> {
    // 最小化的窗口 DWM 不再合成，WGC 一帧也拿不到。与截图一致：先还原再录。
    // 不能无条件 SW_RESTORE——那会把最大化的窗口一并还原掉。
    if recorder::is_minimized(window_id as isize) {
        restore_minimized(window_id)
            .map_err(|error| format!("目标窗口已最小化，且无法还原：{error}"))?;
    }
    let active = recorder::start(
        window_id as isize,
        options.include_cursor,
        options.system_audio,
        options.microphone,
        output,
    )?;
    Ok((Recording(active), None))
}

/// xcap 截不到光标：截完按窗口原点与 DPI 比例把当前系统光标合成上去。
/// 返回的第二项是光标没合成上的原因（截图本身仍然成功）。
pub(crate) fn capture_with_cursor(
    area: &Area,
    capture: impl FnOnce() -> Result<RgbaImage, String>,
) -> Result<(RgbaImage, Option<String>), String> {
    let mut image = capture()?;
    let failure = cursor::overlay_into(&mut image, (area.x, area.y), area.width).err();
    Ok((image, failure))
}

pub(crate) fn open_folder(path: &Path) -> Result<(), String> {
    // explorer.exe 即使成功也常返回非 0，所以只看能不能启动，不看退出码
    Command::new("explorer")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开文件管理器：{error}"))
}

pub(crate) fn capabilities() -> PlatformCapabilities {
    PlatformCapabilities {
        os: "windows".into(),
        display_server: "Win32".into(),
        recording: CapabilityStatus::yes(),
        recording_system_audio: CapabilityStatus::yes(),
        recording_microphone: CapabilityStatus::yes(),
        autostart: CapabilityStatus::yes(),
        scrolling: None,
        include_cursor: CapabilityStatus::yes(),
    }
}
