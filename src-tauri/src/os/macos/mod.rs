//! macOS 平台层。
//!
//! 与 Windows / Linux 平台层对外提供同名的一组函数（见 lib.rs 里 `mod os` 的说明），
//! 共享代码只经 `os::` 调用。实现都用系统原生能力：ScreenCaptureKit 录制
//! （经 Swift sidecar）、Accessibility 还原窗口、LaunchAgent 自启。

mod apps;
mod autostart;
mod cursor;
mod icon;
mod recorder;
mod window;

use crate::capabilities::{CapabilityStatus, PlatformCapabilities};
use crate::capture::Area;
use image::RgbaImage;
use std::{path::Path, process::Command};

pub(crate) use apps::{installed_apps, launch_icon_png, launch_target};
pub(crate) use autostart::apply_launch_on_boot as apply_autostart;
pub(crate) use icon::png_for_pid as app_icon_png;
pub(crate) use recorder::{start_recording, Recording};
pub(crate) use window::restore_minimized;

/// 本平台支持的全局快捷键动作，顺序即设置页的显示顺序
pub(crate) const SHORTCUT_ACTIONS: &[&str] =
    &["snapshot", "fullscreen", "record", "polish", "palette"];

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
    Command::new("open")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开访达：{error}"))
}

fn major_version() -> Option<u32> {
    let output = Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .split('.')
        .next()?
        .parse()
        .ok()
}

pub(crate) fn capabilities() -> PlatformCapabilities {
    let recording = CapabilityStatus::probe(recorder::probe());
    // 系统声音、麦克风都走 ScreenCaptureKit：录不了窗口就都不行；麦克风还要 macOS 15+
    let recording_system_audio = recording;
    let microphone_supported = major_version()
        .map(|version| version >= 15)
        .unwrap_or(false);
    let recording_microphone = if recording.available && microphone_supported {
        CapabilityStatus::yes()
    } else {
        CapabilityStatus::no()
    };
    PlatformCapabilities {
        os: "macos".into(),
        display_server: "AppKit".into(),
        recording,
        recording_system_audio,
        recording_microphone,
        autostart: CapabilityStatus::probe(autostart::autostart_capability()),
        scrolling: None,
        include_cursor: CapabilityStatus::yes(),
    }
}
