//! 开始菜单里的快捷方式。这就是系统「所有应用」那一份：
//! 商店应用只要在开始菜单里有快捷方式，ShellExecute 也能打开。
//! 不扫注册表里的卸载项，那里混着运行库和更新包，不是人会去点的程序。
//!
//! 开始菜单里还混着一批不是拿来点开用的条目：cmd / PowerShell / node / python 这类
//! 控制台程序、事件查看器这类 .msc 管理单元、帮助文档、安装器与更新器，以及控制面板、
//! 运行这样根本没有文件目标的 shell 项。判断依据是快捷方式真正的目标（走 IShellLink
//! 解析，和资源管理器同一条路径）加上目标 PE 头里的子系统，不看名字——名字是本地的，
//! 也会被用户改。
//!
//! 只有明确判定是上面这些的才隐藏。目标没解析出来（COM 一时不可用、快捷方式损坏）
//! 一律保留，免得一次解析失败就把整份清单清空。

use std::{
    collections::HashSet,
    fs,
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::null,
};

use windows::{
    core::{Interface, PCWSTR},
    Win32::{
        System::Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ},
        UI::Shell::{IShellLinkW, ShellLink},
    },
};
use windows_sys::Win32::{
    System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED},
    UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
};

use super::icon;
use crate::launchpad::{stable_id, InstalledApp};

/// PE 可选头里的 Subsystem：3 是控制台程序，2 是图形程序。
const SUBSYSTEM_WINDOWS_GUI: u16 = 2;
const SUBSYSTEM_WINDOWS_CUI: u16 = 3;

/// 候选应用，外加解析出来的目标（None 表示没解析出来）。
struct Candidate {
    app: InstalledApp,
    target: Option<String>,
}

pub(crate) fn installed_apps() -> Vec<InstalledApp> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for root in roots() {
        collect(&root, &mut found, &mut seen, 0);
    }
    found.sort_by(|left, right| {
        left.app
            .name
            .to_lowercase()
            .cmp(&right.app.name.to_lowercase())
            .then(left.app.id.cmp(&right.app.id))
    });

    // 同一个程序在开始菜单里可能有多个快捷方式（比如 Tailscale 顶层和文件夹里各一个），
    // 按解析出来的目标去重，留下排序靠前的那个。
    let mut targets = HashSet::new();
    let mut apps = Vec::with_capacity(found.len());
    for candidate in found {
        if let Some(target) = candidate.target.as_deref() {
            let target = target.trim();
            if !target.is_empty() && !targets.insert(normalize(Path::new(target))) {
                continue;
            }
        }
        apps.push(candidate.app);
    }
    apps
}

pub(crate) fn launch_target(target: &str) -> Result<(), String> {
    let operation = wide("open");
    let file = wide(target);
    let code = unsafe {
        // 快捷方式要走 Shell，公寓线程没初始化时有的机器会直接失败
        let _ = CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32);
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if (code as isize) <= 32 {
        return Err(format!("无法启动这个应用（错误 {}）", code as isize));
    }
    Ok(())
}

pub(crate) fn launch_icon_png(target: &str, size: u32) -> Option<Vec<u8>> {
    icon::png_for_path(Path::new(target), size)
}

fn roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = dirs::data_dir() {
        roots.push(dir.join("Microsoft\\Windows\\Start Menu\\Programs"));
    }
    if let Some(dir) = std::env::var_os("ProgramData") {
        roots.push(PathBuf::from(dir).join("Microsoft\\Windows\\Start Menu\\Programs"));
    }
    roots
}

fn collect(dir: &Path, into: &mut Vec<Candidate>, seen: &mut HashSet<String>, depth: u32) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let raw_name = entry.file_name();
        let raw_name = raw_name.to_string_lossy();
        if raw_name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect(&path, into, seen, depth + 1);
            continue;
        }
        if !raw_name.to_ascii_lowercase().ends_with(".lnk") {
            continue;
        }
        let Some(stem) = path
            .file_stem()
            .map(|value| value.to_string_lossy().trim().to_string())
        else {
            continue;
        };
        if stem.is_empty() || junk(&stem) {
            continue;
        }
        let id = stable_id(&normalize(&path));
        if !seen.insert(id.clone()) {
            continue;
        }
        let target = shortcut_target(&path);
        if !usable(&stem, target.as_deref()) {
            continue;
        }
        into.push(Candidate {
            app: InstalledApp {
                id,
                name: crate::truncate(&stem, 80),
                target: path.to_string_lossy().to_string(),
            },
            target,
        });
    }
}

fn normalize(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

/// 卸载程序不是会打开来用的应用。
fn junk(name: &str) -> bool {
    let name = name.to_lowercase();
    name.contains("uninstall") || name.contains("卸载")
}

/// 明确判定「不是拿来点开用的应用」才返回 false。target 为 None 表示没解析出来，按未知处理，保留。
fn usable(stem: &str, target: Option<&str>) -> bool {
    let Some(target) = target else {
        return true;
    };
    let target = target.trim();
    if target.is_empty() {
        // 控制面板、运行这类 shell 项没有文件目标，点开也不是启动一个应用
        return false;
    }
    let path = Path::new(target);
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if extension != "exe" {
        // .msc 管理单元（事件查看器、服务）、.url / .html / .txt 文档
        return false;
    }
    if installer(stem, path) {
        return false;
    }
    // 只有读出来确实是控制台程序才隐藏；读不出来（文件没了、不是 PE）当未知保留
    subsystem(path) != Some(SUBSYSTEM_WINDOWS_CUI)
}

/// 安装器和更新器：名字带 Installer，或者目标可执行文件本身就是更新程序。
fn installer(stem: &str, target: &Path) -> bool {
    if stem.to_lowercase().ends_with("installer") {
        return true;
    }
    let file = target
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_lowercase();
    file.ends_with("update") || file.ends_with("updater")
}

/// PE 可选头里的 Subsystem 字段。只认图形 / 控制台两种，别的当未知。
fn subsystem(path: &Path) -> Option<u16> {
    let mut file = File::open(path).ok()?;
    let mut dos = [0u8; 0x40];
    file.read_exact(&mut dos).ok()?;
    if &dos[0..2] != b"MZ" {
        return None;
    }
    let pe = u32::from_le_bytes(dos[0x3c..0x40].try_into().ok()?) as u64;
    file.seek(SeekFrom::Start(pe)).ok()?;
    let mut header = [0u8; 0x60];
    file.read_exact(&mut header).ok()?;
    if &header[0..4] != b"PE\0\0" {
        return None;
    }
    let subsystem = u16::from_le_bytes(header[0x5c..0x5e].try_into().ok()?);
    (subsystem == SUBSYSTEM_WINDOWS_GUI || subsystem == SUBSYSTEM_WINDOWS_CUI).then_some(subsystem)
}

/// 快捷方式的真实目标。走 IShellLink 解析，和资源管理器同一条路径：
/// 命令行、事件查看器这类快捷键没有 LinkInfo，光解析文件格式会漏掉。
fn shortcut_target(link: &Path) -> Option<String> {
    unsafe {
        let _ = CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32);
        let shell_link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let persist: IPersistFile = shell_link.cast().ok()?;
        let wide: Vec<u16> = link
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        persist.Load(PCWSTR(wide.as_ptr()), STGM_READ).ok()?;
        let mut buffer = [0u16; 4096];
        // 不要 SLGP_RAWPATH：那个给的是没展开的 %windir%\... ，读不了 PE 头
        shell_link
            .GetPath(&mut buffer, std::ptr::null_mut(), 0)
            .ok()?;
        let len = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        let text = String::from_utf16_lossy(&buffer[..len]);
        Some(expand(&text))
    }
}

/// 兜底展开 %VAR%：正常路径下 GetPath 已经展开过，这里防的是少数没被展开的快捷方式。
fn expand(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => match std::env::var(&after[..end]) {
                Ok(value) => {
                    out.push_str(&value);
                    rest = &after[end + 1..];
                }
                Err(_) => {
                    out.push('%');
                    rest = after;
                }
            },
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_menu_lists_shortcuts_without_duplicates() {
        let apps = installed_apps();
        assert!(!apps.is_empty(), "开始菜单里应该能找到快捷方式");
        let mut ids = HashSet::new();
        for app in &apps {
            assert!(!app.name.trim().is_empty());
            assert!(
                app.target.to_ascii_lowercase().ends_with(".lnk"),
                "{}",
                app.target
            );
            assert!(ids.insert(app.id.clone()), "重复的应用：{}", app.name);
        }
    }

    #[test]
    fn most_shortcuts_have_png_icons() {
        let apps = installed_apps();
        let mut ok = 0usize;
        let mut missing = Vec::new();
        for app in &apps {
            match launch_icon_png(&app.target, 32) {
                Some(bytes) => {
                    assert!(bytes.starts_with(b"\x89PNG"), "{}", app.name);
                    ok += 1;
                }
                None if missing.len() < 6 => missing.push(app.name.clone()),
                None => {}
            }
        }
        assert!(
            ok * 4 >= apps.len() * 3,
            "只有 {ok}/{} 个快捷方式取出了图标，例如 {}",
            apps.len(),
            missing.join("、")
        );
    }

    /// 同一份名单里不该出现同一个可执行文件两次（Tailscale 有两个快捷方式）。
    #[test]
    fn no_target_appears_twice() {
        let apps = installed_apps();
        let mut targets = HashSet::new();
        for app in &apps {
            if let Some(target) = shortcut_target(Path::new(&app.target)) {
                let target = target.trim();
                if target.is_empty() {
                    continue;
                }
                assert!(
                    targets.insert(normalize(Path::new(target))),
                    "{} 出现了两次",
                    app.name
                );
            }
        }
    }

    /// 控制台程序、文档、管理单元和安装器不进启动台；读不出来的目标保留。
    #[test]
    fn console_documents_and_installers_stay_out() {
        assert!(usable(
            "Blender 5.2",
            Some(r"C:\Program Files\Blender\blender-launcher.exe")
        ));
        assert!(!usable(
            "Event Viewer",
            Some(r"C:\Windows\System32\eventvwr.msc")
        ));
        assert!(!usable(
            "License",
            Some(r"C:\Program Files (x86)\Dev-Cpp\COPYING.txt")
        ));
        assert!(!usable("File Explorer", Some("")));
        assert!(usable("File Explorer", None), "解析不出来时按未知保留");
        assert!(!usable("Visual Studio Installer", Some(r"C:\VS\setup.exe")));
        assert!(!usable(
            "微信输入法",
            Some(r"C:\Tencent\WeType\wetype_update.exe")
        ));
    }
}
