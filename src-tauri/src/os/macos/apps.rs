//! /Applications 和用户自己的「应用程序」文件夹。不进 .app 包内部，
//! 否则会把每个应用自带的辅助工具也列出来。

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use objc2_app_kit::NSWorkspace;
use objc2_foundation::NSString;

use super::icon;
use crate::launchpad::{stable_id, InstalledApp};

pub(crate) fn installed_apps() -> Vec<InstalledApp> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Applications"));
    }
    for root in roots {
        walk(&root, 0, &mut found, &mut seen);
    }
    found.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then(left.id.cmp(&right.id))
    });
    found
}

pub(crate) fn launch_target(target: &str) -> Result<(), String> {
    Command::new("open")
        .arg(target)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法启动这个应用：{error}"))
}

pub(crate) fn launch_icon_png(target: &str, size: u32) -> Option<Vec<u8>> {
    unsafe {
        let workspace = NSWorkspace::sharedWorkspace();
        let icon = workspace.iconForFile(&NSString::from_str(target));
        icon::png_of(&icon, size)
    }
}

fn walk(dir: &Path, depth: u32, into: &mut Vec<InstalledApp>, seen: &mut HashSet<String>) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        if name.to_ascii_lowercase().ends_with(".app") {
            push_app(&path, into, seen);
            continue;
        }
        if path.is_dir() {
            walk(&path, depth + 1, into, seen);
        }
    }
}

fn push_app(path: &Path, into: &mut Vec<InstalledApp>, seen: &mut HashSet<String>) {
    let fallback = path
        .file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "应用".into());
    let plist = fs::read_to_string(path.join("Contents/Info.plist")).unwrap_or_default();
    let text = if plist.starts_with("bplist") {
        String::new()
    } else {
        plist
    };
    let name = plist_string(&text, "CFBundleDisplayName")
        .or_else(|| plist_string(&text, "CFBundleName"))
        .unwrap_or(fallback);
    let id = plist_string(&text, "CFBundleIdentifier")
        .unwrap_or_else(|| stable_id(&path.to_string_lossy()));
    if name.trim().is_empty() || !seen.insert(id.clone()) {
        return;
    }
    into.push(InstalledApp {
        id,
        name: crate::truncate(name.trim(), 80),
        target: path.to_string_lossy().to_string(),
        listed: true,
    });
}

fn plist_string(xml: &str, key: &str) -> Option<String> {
    let needle = format!("<key>{key}</key>");
    let rest = xml.split_once(&needle)?.1;
    let start = rest.find("<string>")? + "<string>".len();
    let end = start + rest[start..].find("</string>")?;
    let value = rest[start..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}
