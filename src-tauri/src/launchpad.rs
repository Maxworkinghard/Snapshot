//! 启动台：本机已安装的应用，外加用户收成的文件夹。
//!
//! 应用清单每次打开时向系统要，不写进配置。配置里只留用户排过的顺序和文件夹，
//! 这样新装的应用会自己出现在末尾，卸掉的会从文件夹里消失。

use super::{actions, os, settings, AppState};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State};

#[derive(Clone, Debug)]
pub(crate) struct InstalledApp {
    pub(crate) id: String,
    pub(crate) name: String,
    /// 平台自己的启动目标（快捷方式、.app、.desktop）。不发给页面。
    pub(crate) target: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LaunchpadFolder {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) app_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppName {
    id: String,
    name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum LaunchpadItem {
    App {
        id: String,
        name: String,
    },
    Folder {
        id: String,
        name: String,
        apps: Vec<AppName>,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LaunchpadView {
    apps: Vec<AppName>,
    items: Vec<LaunchpadItem>,
}

/// 路径或包名的稳定短 id。只为媒体地址里不出现斜杠，不是安全边界。
pub(crate) fn stable_id(key: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn clean_name(name: &str) -> String {
    let trimmed = name.trim();
    super::truncate(
        if trimmed.is_empty() {
            "未命名"
        } else {
            trimmed
        },
        24,
    )
}

/// 用当前安装清单解释用户保存的顺序。卸掉的应用去掉，不满两个的文件夹解散回应用。
fn arrange(
    apps: &[InstalledApp],
    order: &[String],
    folders: &[LaunchpadFolder],
) -> (Vec<LaunchpadItem>, Vec<String>, Vec<LaunchpadFolder>) {
    let known: HashMap<&str, &str> = apps
        .iter()
        .map(|app| (app.id.as_str(), app.name.as_str()))
        .collect();
    let mut used = HashSet::new();
    let mut folder_by_id: HashMap<String, LaunchpadFolder> = HashMap::new();
    let mut folder_apps: HashMap<String, Vec<AppName>> = HashMap::new();
    let mut dissolved: HashMap<String, String> = HashMap::new();

    let mut seen_folders = HashSet::new();
    for folder in folders {
        if folder.id.is_empty() || !seen_folders.insert(folder.id.clone()) {
            continue;
        }
        let mut members = Vec::new();
        for id in &folder.app_ids {
            if !known.contains_key(id.as_str()) || !used.insert(id.clone()) {
                continue;
            }
            members.push(AppName {
                id: id.clone(),
                name: known[id.as_str()].to_string(),
            });
        }
        if members.len() >= 2 {
            let stored = LaunchpadFolder {
                id: folder.id.clone(),
                name: clean_name(&folder.name),
                app_ids: members.iter().map(|app| app.id.clone()).collect(),
            };
            folder_by_id.insert(folder.id.clone(), stored);
            folder_apps.insert(folder.id.clone(), members);
        } else {
            for app in &members {
                used.remove(&app.id);
            }
            if let Some(only) = members.pop() {
                dissolved.insert(folder.id.clone(), only.id);
            }
        }
    }

    let mut items = Vec::new();
    let mut emitted_folders = HashSet::new();
    let mut seen_tokens = HashSet::new();
    for token in order {
        if !seen_tokens.insert(token.clone()) {
            continue;
        }
        let Some((kind, id)) = token.split_once(':') else {
            continue;
        };
        if kind == "folder" {
            if let Some(folder) = folder_by_id.get(id) {
                items.push(LaunchpadItem::Folder {
                    id: folder.id.clone(),
                    name: folder.name.clone(),
                    apps: folder_apps.get(id).cloned().unwrap_or_default(),
                });
                emitted_folders.insert(id.to_string());
            } else if let Some(app_id) = dissolved.get(id) {
                push_app(&mut items, &mut used, &known, app_id);
            }
        } else if kind == "app" {
            push_app(&mut items, &mut used, &known, id);
        }
    }

    for folder in folders {
        if folder_by_id.contains_key(&folder.id) && emitted_folders.insert(folder.id.clone()) {
            let stored = &folder_by_id[&folder.id];
            items.push(LaunchpadItem::Folder {
                id: stored.id.clone(),
                name: stored.name.clone(),
                apps: folder_apps.get(&folder.id).cloned().unwrap_or_default(),
            });
        }
    }

    let mut rest: Vec<&InstalledApp> = apps.iter().filter(|app| !used.contains(&app.id)).collect();
    rest.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then(left.id.cmp(&right.id))
    });
    for app in rest {
        used.insert(app.id.clone());
        items.push(LaunchpadItem::App {
            id: app.id.clone(),
            name: app.name.clone(),
        });
    }

    let mut stored_order = Vec::new();
    let mut stored_folders = Vec::new();
    for item in &items {
        match item {
            LaunchpadItem::App { id, .. } => stored_order.push(format!("app:{id}")),
            LaunchpadItem::Folder { id, name, apps } => {
                stored_order.push(format!("folder:{id}"));
                stored_folders.push(LaunchpadFolder {
                    id: id.clone(),
                    name: name.clone(),
                    app_ids: apps.iter().map(|app| app.id.clone()).collect(),
                });
            }
        }
    }
    (items, stored_order, stored_folders)
}

fn push_app(
    items: &mut Vec<LaunchpadItem>,
    used: &mut HashSet<String>,
    known: &HashMap<&str, &str>,
    id: &str,
) {
    if used.contains(id) {
        return;
    }
    let Some(name) = known.get(id) else {
        return;
    };
    used.insert(id.to_string());
    items.push(LaunchpadItem::App {
        id: id.to_string(),
        name: (*name).to_string(),
    });
}

fn view_from(apps: &[InstalledApp], items: Vec<LaunchpadItem>) -> LaunchpadView {
    LaunchpadView {
        apps: apps
            .iter()
            .map(|app| AppName {
                id: app.id.clone(),
                name: app.name.clone(),
            })
            .collect(),
        items,
    }
}

fn refresh_catalog(state: &AppState) -> Vec<InstalledApp> {
    let apps = os::installed_apps();
    *state.launch_catalog.lock() = apps.clone();
    apps
}

fn target_of(state: &AppState, id: &str) -> Option<String> {
    if let Some(target) = state
        .launch_catalog
        .lock()
        .iter()
        .find(|app| app.id == id)
        .map(|app| app.target.clone())
    {
        return Some(target);
    }
    let apps = refresh_catalog(state);
    apps.into_iter()
        .find(|app| app.id == id)
        .map(|app| app.target)
}

pub(crate) fn icon_bytes(state: &AppState, id: &str, size: u32) -> Result<Vec<u8>, String> {
    let size = size.clamp(16, 256);
    let key = format!("{id}:{size}");
    if let Some(cached) = state.launch_icons.lock().get(&key) {
        return Ok(cached.clone());
    }
    let target = target_of(state, id).ok_or_else(|| "没有这个应用的图标".to_string())?;
    let bytes =
        os::launch_icon_png(&target, size).ok_or_else(|| "没有这个应用的图标".to_string())?;
    state.launch_icons.lock().insert(key, bytes.clone());
    Ok(bytes)
}

fn monitor_for(window: &tauri::WebviewWindow, x: f64, y: f64) -> Option<tauri::Monitor> {
    let scale = window.scale_factor().unwrap_or(1.0);
    let (sx, sy) = (x * scale, y * scale);
    let monitors = window.available_monitors().ok()?;
    monitors
        .into_iter()
        .find(|monitor| {
            let origin = monitor.position();
            let size = monitor.size();
            sx >= origin.x as f64
                && sx < (origin.x + size.width as i32) as f64
                && sy >= origin.y as f64
                && sy < (origin.y + size.height as i32) as f64
        })
        .or_else(|| window.primary_monitor().ok().flatten())
}

/// 双击桌宠时盖住它所在的那块屏幕。桌宠先让开，否则两个置顶窗口会叠在一起。
#[tauri::command]
pub(crate) fn show_launchpad(app: AppHandle, x: f64, y: f64) -> Result<(), String> {
    actions::hide_quick_menu(app.clone());
    let window = app
        .get_webview_window("launchpad")
        .ok_or_else(|| "启动台窗口不存在".to_string())?;
    if let Some(monitor) = monitor_for(&window, x, y) {
        let origin = monitor.position();
        let size = monitor.size();
        let _ = window.set_position(PhysicalPosition::new(origin.x, origin.y));
        let _ = window.set_size(PhysicalSize::new(size.width, size.height));
    }
    if let Some(pet) = app.get_webview_window("pet") {
        let _ = pet.set_always_on_top(false);
    }
    let _ = window.set_always_on_top(true);
    if let Err(error) = window.show() {
        conceal(&app);
        return Err(error.to_string());
    }
    let _ = window.set_focus();
    let _ = app.emit_to("launchpad", "launchpad-opened", ());
    Ok(())
}

pub(crate) fn conceal(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("launchpad") {
        let _ = window.hide();
        let _ = window.set_always_on_top(false);
    }
    if let Some(pet) = app.get_webview_window("pet") {
        let _ = pet.set_always_on_top(true);
    }
}

#[tauri::command]
pub(crate) fn hide_launchpad(app: AppHandle) {
    conceal(&app);
}

#[tauri::command]
pub(crate) fn launchpad_state(state: State<'_, AppState>) -> LaunchpadView {
    let apps = refresh_catalog(&state);
    let (order, folders) = {
        let settings = state.settings.lock();
        (
            settings.launchpad_order.clone(),
            settings.launchpad_folders.clone(),
        )
    };
    let (items, _, _) = arrange(&apps, &order, &folders);
    view_from(&apps, items)
}

#[tauri::command]
pub(crate) fn save_launchpad(
    app: AppHandle,
    state: State<'_, AppState>,
    order: Vec<String>,
    folders: Vec<LaunchpadFolder>,
) -> Result<(), String> {
    let apps = {
        let catalog = state.launch_catalog.lock().clone();
        if catalog.is_empty() {
            refresh_catalog(&state)
        } else {
            catalog
        }
    };
    let (_, order, folders) = arrange(&apps, &order, &folders);
    let saved = {
        let mut settings = state.settings.lock();
        settings.launchpad_order = order;
        settings.launchpad_folders = folders;
        settings::persist_settings(&state.settings_path, &settings)?;
        settings.clone()
    };
    settings::emit_settings(&app, &saved);
    Ok(())
}

#[tauri::command]
pub(crate) fn launch_app(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let target = target_of(&state, &id).ok_or_else(|| "这个应用已经不在了".to_string())?;
    os::launch_target(&target)?;
    conceal(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, name: &str) -> InstalledApp {
        InstalledApp {
            id: id.into(),
            name: name.into(),
            target: id.into(),
        }
    }

    fn ids(items: &[LaunchpadItem]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                LaunchpadItem::App { id, .. } => format!("app:{id}"),
                LaunchpadItem::Folder { id, apps, .. } => {
                    let members: Vec<_> = apps.iter().map(|app| app.id.as_str()).collect();
                    format!("folder:{id}[{}]", members.join(","))
                }
            })
            .collect()
    }

    #[test]
    fn fresh_layout_lists_every_app_by_name() {
        let apps = vec![app("b", "记事本"), app("a", "Edge")];
        let (items, _, _) = arrange(&apps, &[], &[]);
        assert_eq!(ids(&items), ["app:a", "app:b"]);
    }

    #[test]
    fn folder_hides_its_apps_and_a_lone_member_dissolves() {
        let apps = vec![app("a", "A"), app("b", "B"), app("c", "C"), app("d", "D")];
        let folders = vec![
            LaunchpadFolder {
                id: "f1".into(),
                name: " 办公 ".into(),
                app_ids: vec!["b".into(), "c".into(), "missing".into()],
            },
            LaunchpadFolder {
                id: "f2".into(),
                name: "".into(),
                app_ids: vec!["a".into()],
            },
        ];
        let order = vec![
            "folder:f2".into(),
            "folder:f1".into(),
            "app:d".into(),
            "app:nope".into(),
        ];
        let (items, stored_order, stored_folders) = arrange(&apps, &order, &folders);
        assert_eq!(ids(&items), ["app:a", "folder:f1[b,c]", "app:d"]);
        assert_eq!(stored_order, ["app:a", "folder:f1", "app:d"]);
        assert_eq!(stored_folders.len(), 1);
        assert_eq!(stored_folders[0].name, "办公");
        assert_eq!(stored_folders[0].app_ids, ["b", "c"]);
    }

    #[test]
    fn an_app_only_stays_in_the_first_folder() {
        let apps = vec![app("a", "A"), app("b", "B"), app("c", "C")];
        let folders = vec![
            LaunchpadFolder {
                id: "f1".into(),
                name: "一".into(),
                app_ids: vec!["a".into(), "b".into()],
            },
            LaunchpadFolder {
                id: "f2".into(),
                name: "二".into(),
                app_ids: vec!["b".into(), "c".into()],
            },
        ];
        let (items, _, _) = arrange(&apps, &["folder:f1".into(), "folder:f2".into()], &folders);
        assert_eq!(ids(&items), ["folder:f1[a,b]", "app:c"]);
    }
}
