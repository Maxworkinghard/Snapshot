//! Linux 平台层。
//!
//! 与 Windows / macOS 平台层对外提供同名的一组函数（见 lib.rs 里 `mod os` 的说明），
//! 共享代码只经 `os::` 调用。成熟的系统能力直接用（ffmpeg / xdotool / XDG / Freedesktop /
//! portal），不为了语言统一重写。不可用时返回可读错误，不静默假装成功。

mod apps;
mod autostart;
mod icon;
mod recording;
mod scrolling;
mod session;
mod still;
mod window;

use crate::capabilities::{CapabilityStatus, PlatformCapabilities};
use crate::capture::Area;
use crate::recording::RecordOptions;
use image::RgbaImage;
use std::{path::Path, process::Command};

pub(crate) use apps::{installed_apps, launch_icon_png, launch_target};
pub(crate) use autostart::apply_launch_on_boot as apply_autostart;
pub(crate) use scrolling::capture_scrolling_window;
pub(crate) use window::restore_minimized;

/// 本平台支持的全局快捷键动作，顺序即设置页的显示顺序（滚动长截图只有 Linux/X11 有）
pub(crate) const SHORTCUT_ACTIONS: &[&str] = &[
    "snapshot",
    "fullscreen",
    "scrolling",
    "record",
    "polish",
    "palette",
];

/// 一次进行中的录制（ffmpeg 子进程 + 可选 portal）
pub(crate) struct Recording(recording::ActiveRecording);

impl Recording {
    /// ffmpeg 中途退出目前不单独检测，停止时一并收尾
    pub(crate) fn exit_reason(&mut self) -> Option<String> {
        None
    }

    pub(crate) fn stop(self) -> Result<(), String> {
        self.0.stop();
        Ok(())
    }
}

/// 第二个返回值是启动时给界面的提示（portal 需用户重新选窗/屏）。
pub(crate) fn start_recording(
    window_id: u32,
    options: &RecordOptions,
    output: &Path,
) -> Result<(Recording, Option<String>), String> {
    let (active, message) = recording::start_recording(
        window_id,
        options.include_cursor,
        options.system_audio,
        options.microphone,
        output,
    )?;
    Ok((Recording(active), message))
}

/// X11 下让 ffmpeg x11grab 直接带光标抓这块矩形；抓不了就退回不带光标的截法，
/// 返回的第二项是原因（截图本身仍然成功）。
pub(crate) fn capture_with_cursor(
    area: &Area,
    capture: impl FnOnce() -> Result<RgbaImage, String>,
) -> Result<(RgbaImage, Option<String>), String> {
    match still::capture_region_with_cursor(area.x, area.y, area.width, area.height) {
        Ok(image) => Ok((image, None)),
        Err(error) => Ok((capture()?, Some(error))),
    }
}

/// 进程图标，编码成 PNG（媒体协议 icon/<pid>/<边长> 用）。
pub(crate) fn app_icon_png(pid: u32, size: u32) -> Option<Vec<u8>> {
    crate::capture::encode_png(&icon::icon_for_process(pid, size)?).ok()
}

pub(crate) fn open_folder(path: &Path) -> Result<(), String> {
    Command::new("xdg-open")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开文件管理器：{error}"))
}

pub(crate) fn capabilities() -> PlatformCapabilities {
    // 滚动长截图要 X11，翻页靠 xdotool；纯 Wayland 做不了，没装 xdotool 也做不了
    let scrolling = if session::is_wayland_session() || session::x11_display().is_none() {
        CapabilityStatus::no()
    } else {
        CapabilityStatus::probe(scrolling::ensure_xdotool())
    };
    PlatformCapabilities {
        os: "linux".into(),
        display_server: session::display_server_label().into(),
        recording: CapabilityStatus::probe(recording::recording_available()),
        recording_system_audio: recording::system_audio_capability(),
        recording_microphone: recording::microphone_capability(),
        autostart: CapabilityStatus::probe(autostart::autostart_capability()),
        scrolling: Some(scrolling),
        include_cursor: CapabilityStatus::yes(),
    }
}
