//! XDG `.desktop` 文件。用户目录盖过系统目录里的同名 id，和桌面环境的规则一致。
//! `NoDisplay` / `Hidden` 的（卸载器、打开方式）不进启动台。

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::launchpad::InstalledApp;

pub(crate) fn installed_apps() -> Vec<InstalledApp> {
    let mut by_id = HashMap::new();
    for dir in app_dirs() {
        collect(&dir, &mut by_id, 0);
    }
    let mut found: Vec<_> = by_id.into_values().collect();
    found.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then(left.id.cmp(&right.id))
    });
    found
}

pub(crate) fn launch_target(target: &str) -> Result<(), String> {
    if Command::new("gio").args(["launch", target]).spawn().is_ok() {
        return Ok(());
    }
    let text = fs::read_to_string(target).map_err(|_| "这个应用的启动项已经不在了".to_string())?;
    let fields = parse_desktop(&text);
    let exec = fields.get("Exec").map(String::as_str).unwrap_or("");
    let mut args = split_exec(exec);
    if args.is_empty() {
        return Err("这个应用没有可执行的启动命令".into());
    }
    let program = args.remove(0);
    Command::new(program)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法启动这个应用：{error}"))
}

pub(crate) fn launch_icon_png(target: &str, _size: u32) -> Option<Vec<u8>> {
    let text = fs::read_to_string(target).ok()?;
    let name = parse_desktop(&text).get("Icon")?.to_string();
    let path = find_icon(&name)?;
    let bytes = fs::read(&path).ok()?;
    if path.extension().and_then(|ext| ext.to_str()) == Some("png") {
        return Some(bytes);
    }
    let image = image::load_from_memory(&bytes).ok()?;
    crate::capture::encode_png(&image.to_rgba8()).ok()
}

/// 系统目录在前，用户目录在后：同名 id 后来的盖掉先前的。
fn app_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let data_dirs =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    for dir in data_dirs.split(':') {
        if !dir.is_empty() {
            dirs.push(PathBuf::from(dir).join("applications"));
        }
    }
    if let Some(home) = dirs::home_dir() {
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"));
        dirs.push(data.join("applications"));
    }
    dirs
}

fn collect(dir: &Path, into: &mut HashMap<String, InstalledApp>, depth: u32) {
    if depth > 2 {
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
        if path.is_dir() {
            collect(&path, into, depth + 1);
            continue;
        }
        if !name.to_ascii_lowercase().ends_with(".desktop") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let fields = parse_desktop(&text);
        if !shown(&fields) {
            continue;
        }
        let id = path
            .file_stem()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_default();
        let app_name = localized_name(&fields);
        if id.is_empty() || app_name.is_empty() {
            continue;
        }
        into.insert(
            id.clone(),
            InstalledApp {
                id,
                name: crate::truncate(&app_name, 80),
                target: path.to_string_lossy().to_string(),
            },
        );
    }
}

fn shown(fields: &HashMap<String, String>) -> bool {
    if flag(fields, "NoDisplay") || flag(fields, "Hidden") {
        return false;
    }
    if fields
        .get("Type")
        .map(String::as_str)
        .unwrap_or("Application")
        != "Application"
    {
        return false;
    }
    if fields
        .get("Exec")
        .map(|value| value.trim().is_empty())
        .unwrap_or(true)
    {
        return false;
    }
    if let Some(try_exec) = fields.get("TryExec") {
        if !try_exec.is_empty() && !executable_exists(try_exec) {
            return false;
        }
    }
    true
}

fn flag(fields: &HashMap<String, String>, key: &str) -> bool {
    fields
        .get(key)
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn localized_name(fields: &HashMap<String, String>) -> String {
    let lang = std::env::var("LANG").unwrap_or_default();
    let lang = lang.split(['.', '@']).next().unwrap_or("");
    let short = lang.split('_').next().unwrap_or("");
    for key in [
        format!("Name[{lang}]"),
        format!("Name[{short}]"),
        "Name".into(),
    ] {
        if let Some(value) = fields.get(&key) {
            let value = value.trim();
            if !value.is_empty() {
                return value.to_string();
            }
        }
    }
    String::new()
}

fn executable_exists(name: &str) -> bool {
    if name.contains('/') {
        return Path::new(name).is_file();
    }
    let Some(path) = std::env::var_os("PATH") else {
        return true;
    };
    std::env::split_paths(&path).any(|dir| dir.join(name).is_file())
}

fn parse_desktop(text: &str) -> HashMap<String, String> {
    let mut in_entry = false;
    let mut fields = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix('[') {
            in_entry = rest
                .trim_end_matches(']')
                .eq_ignore_ascii_case("Desktop Entry");
            continue;
        }
        if !in_entry || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        fields.insert(key.trim().to_string(), value.trim().to_string());
    }
    fields
}

/// 去掉 %f %U 这类由文件管理器填的占位。启动台是直接打开应用，没有文件可填。
fn split_exec(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for ch in exec.chars() {
        match quote {
            Some(mark) if ch == mark => quote = None,
            Some(_) => current.push(ch),
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    push_field(&mut args, &current);
                    current.clear();
                }
            }
            None => current.push(ch),
        }
    }
    if !current.is_empty() {
        push_field(&mut args, &current);
    }
    args
}

fn push_field(args: &mut Vec<String>, field: &str) {
    if field == "%%" {
        args.push("%".into());
        return;
    }
    if field.starts_with('%') && field.chars().count() == 2 {
        return;
    }
    let cleaned = field.replace("%%", "%");
    if !cleaned.is_empty() {
        args.push(cleaned);
    }
}

fn find_icon(name: &str) -> Option<PathBuf> {
    let direct = PathBuf::from(name);
    if direct.is_absolute() && direct.is_file() {
        return Some(direct);
    }
    let file = name.trim_end_matches(".png").trim_end_matches(".svg");
    let mut roots = Vec::new();
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".local/share/icons"));
        roots.push(home.join(".icons"));
    }
    if let Ok(data) = std::env::var("XDG_DATA_DIRS") {
        for dir in data.split(':') {
            if !dir.is_empty() {
                roots.push(PathBuf::from(dir).join("icons"));
            }
        }
    }
    roots.push(PathBuf::from("/usr/share/icons"));
    roots.push(PathBuf::from("/usr/share/pixmaps"));
    let sizes = ["256x256", "128x128", "96x96", "64x64", "48x48", "32x32"];
    let themes = ["hicolor", "Adwaita", "Papirus", "breeze", "Yaru"];
    for root in &roots {
        let pix = root.join(format!("{file}.png"));
        if pix.is_file() {
            return Some(pix);
        }
        for theme in &themes {
            for size in &sizes {
                let candidate = root
                    .join(theme)
                    .join(size)
                    .join("apps")
                    .join(format!("{file}.png"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}
