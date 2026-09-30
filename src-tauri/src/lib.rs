mod actions;
mod activity;
mod capabilities;
mod capture;
mod clipboard;
mod launchpad;
mod media;
mod pet;
mod pet_key;
mod pet_walk;
mod polish;
mod recording;
mod settings;
mod shortcuts;
mod snapshots;
mod tracker;

// 三端各一套原生实现（os/windows、os/macos、os/linux），对外提供同名的一组函数：
// 录制、带光标截图、还原最小化、开机自启、取图标、打开文件夹、本机能力、
// 支持的快捷键动作。共享代码只写 `os::xxx`，不再到处 #[cfg]；
// 某一端少实现了哪个，那一端的编译直接报错，而不是运行时才发现。
#[cfg(target_os = "windows")]
#[path = "os/windows/mod.rs"]
mod os;
#[cfg(target_os = "macos")]
#[path = "os/macos/mod.rs"]
mod os;
#[cfg(target_os = "linux")]
#[path = "os/linux/mod.rs"]
mod os;
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
compile_error!("应用快照只支持 Windows、macOS 和 Linux");

use arboard::{Clipboard, ImageData};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chrono::Local;
use image::{ImageFormat, RgbaImage};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    borrow::Cow,
    collections::HashMap,
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, State, WindowEvent,
};
use xcap::{Monitor, Window};

struct AppState {
    settings_path: PathBuf,
    snapshots_dir: PathBuf,
    /// 快照缩略图，放在应用数据目录，不往用户选的保存目录里塞东西
    thumbs_dir: PathBuf,
    /// 导入时复制进来的伴侣素材与它们的缩略图
    pets_dir: PathBuf,
    activity: Mutex<activity::ActivityLog>,
    /// 剪贴板里是不是我们放的图、多久后清
    clipboard: Mutex<Option<clipboard::ClipboardState>>,
    /// 每放一次图、每次手动清空/取消都加一，让过期的清空线程自己作废
    clipboard_generation: AtomicU64,
    settings: Mutex<settings::Settings>,
    tracker: Arc<Mutex<tracker::TrackerState>>,
    recorder: Mutex<recording::Recorder>,
    pet_position_revision: AtomicU64,
    quick_menu_anchor: Mutex<Option<(f64, f64)>>,
    /// 标注窗口待编辑 PNG（RGBA 编码前的原始 PNG 字节）
    annotate_png: Mutex<Option<Vec<u8>>>,
    annotate_title: Mutex<String>,
    /// 启动时没能注册上的全局快捷键，等主窗口加载后再提示用户
    shortcut_conflicts: Mutex<Vec<String>>,
    launch_catalog: Mutex<Vec<launchpad::InstalledApp>>,
    launch_icons: Mutex<HashMap<String, Vec<u8>>>,
}

pub(crate) fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.into();
    }
    value.chars().take(max).collect::<String>() + "…"
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn run() {
    // 配置文件里定义的窗口在 setup() 之前就已创建并开始加载前端，
    // 若把 manage() 留在 setup() 里，前端可能抢先发出命令并撞上
    // "state not managed"。所以状态提到 Builder 阶段准备好。
    //
    // 路径这里自己算：Tauri 的 app_config_dir()/app_data_dir() 实现就是
    // dirs::config_dir()/dirs::data_dir() 再拼 bundle identifier，
    // 标识符从 context 取，和 tauri.conf.json 保持同源，不会写死漂移。
    let context = tauri::generate_context!();
    let identifier = context.config().identifier.clone();
    let settings_path = dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(&identifier)
        .join("settings.json");
    let data_dir = dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(&identifier);
    let snapshots_dir = data_dir.join("snapshots");
    let thumbs_dir = data_dir.join("thumbs");
    let pets_dir = data_dir.join("pets");
    let activity_path = settings_path.with_file_name("activity.json");
    let settings = settings::read_settings(&settings_path);

    tauri::Builder::default()
        // 必须第一个注册：WebView2 的用户数据目录是独占锁，
        // 第二个实例抢不到就会静默退出（用户看到的是"双击没反应"）。
        // 交给这个插件拦下来，改成把已有窗口唤到前台。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            actions::focus_existing_window(app);
        }))
        .plugin(shortcuts::plugin())
        .plugin(tauri_plugin_dialog::init())
        .register_asynchronous_uri_scheme_protocol(media::SCHEME, media::handle)
        .manage(AppState {
            settings_path,
            snapshots_dir,
            thumbs_dir,
            pets_dir,
            activity: Mutex::new(activity::ActivityLog::load(activity_path)),
            clipboard: Mutex::new(None),
            clipboard_generation: AtomicU64::new(0),
            settings: Mutex::new(settings),
            tracker: Arc::new(Mutex::new(tracker::TrackerState::default())),
            recorder: Mutex::new(recording::Recorder::default()),
            pet_position_revision: AtomicU64::new(0),
            quick_menu_anchor: Mutex::new(None),
            annotate_png: Mutex::new(None),
            annotate_title: Mutex::new(String::new()),
            shortcut_conflicts: Mutex::new(Vec::new()),
            launch_catalog: Mutex::new(Vec::new()),
            launch_icons: Mutex::new(HashMap::new()),
        })
        .setup(|app| {
            let state = app.state::<AppState>();
            let settings = state.settings.lock().clone();
            let tracker = state.tracker.clone();
            if let Some(window) = app.get_webview_window("pet") {
                let window_size = window.outer_size().ok();
                let monitors = window.available_monitors().unwrap_or_default();
                let saved = settings.pet_position.clone().filter(|saved| {
                    monitors.iter().any(|monitor| {
                        let origin = monitor.position();
                        let size = monitor.size();
                        let width = window_size
                            .as_ref()
                            .map(|value| value.width as i32)
                            .unwrap_or(60);
                        let height = window_size
                            .as_ref()
                            .map(|value| value.height as i32)
                            .unwrap_or(60);
                        saved.x >= origin.x
                            && saved.y >= origin.y
                            && saved.x + width <= origin.x + size.width as i32
                            && saved.y + height <= origin.y + size.height as i32
                    })
                });
                if let Some(position) = saved {
                    let _ = window.set_position(PhysicalPosition::new(position.x, position.y));
                } else if let Ok(Some(monitor)) = window.primary_monitor() {
                    let screen = monitor.size();
                    let origin = monitor.position();
                    let width = window_size
                        .as_ref()
                        .map(|value| value.width as i32)
                        .unwrap_or(60);
                    let height = window_size
                        .as_ref()
                        .map(|value| value.height as i32)
                        .unwrap_or(60);
                    let x = origin.x + screen.width as i32 - width - 32;
                    let y = origin.y + (screen.height as i32 - height) / 2;
                    let _ = window.set_position(PhysicalPosition::new(x, y));
                }
            }
            tracker::start_tracker(app.handle().clone(), tracker);

            let failed = shortcuts::register_all(app.handle(), &settings.shortcuts);
            if !failed.is_empty() {
                eprintln!("snapshot: {}", shortcuts::conflict_message(&failed));
                activity::record_error(app.handle(), "快捷键没注册上", &failed.join("、"));
                *state.shortcut_conflicts.lock() = failed;
            }

            // 配置里若已勾选自启，启动时把系统启动项与设置对齐（不会默认打开）。
            // 应用被移动过路径时，这一步顺带把启动项里的旧路径刷新掉。
            if settings.launch_on_boot {
                if let Err(error) = os::apply_autostart(true) {
                    eprintln!("snapshot: could not sync autostart entry: {error}");
                }
            }

            let open_settings =
                MenuItem::with_id(app, "open-settings", "打开设置", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_settings, &quit])?;
            let mut tray = TrayIconBuilder::new()
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open-settings" => actions::show_main_window(app.clone()),
                    "quit" => {
                        if let Some(state) = app.try_state::<AppState>() {
                            let _ = state.recorder.lock().stop();
                        }
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    // Left-click (and Windows double-click) opens the main window where the
                    // tray backend emits click events. Linux tray-icon 0.24 via
                    // libayatana-appindicator has no Activate/click callback — menu only.
                    // Right-click / context menu (打开设置 / 退出) is unchanged.
                    match event {
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                        | TrayIconEvent::DoubleClick {
                            button: MouseButton::Left,
                            ..
                        } => {
                            actions::show_main_window(tray.app_handle().clone());
                        }
                        _ => {}
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            // Linux 上若会话没有 StatusNotifierHost（精简环境 / 无扩展的 GNOME），
            // 托盘会建失败；主窗口与快捷键仍应可用，不能把整个 setup 拖死。
            if let Err(error) = tray.build(app) {
                eprintln!("snapshot: system tray unavailable ({error}); continuing without tray");
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            } else if window.label() == "launchpad" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    launchpad::conceal(window.app_handle());
                }
            } else if window.label() == "pet" {
                if let WindowEvent::Moved(position) = event {
                    let app = window.app_handle().clone();
                    let revision = {
                        let Some(state) = app.try_state::<AppState>() else {
                            return;
                        };
                        state.settings.lock().pet_position = Some(settings::PetPosition {
                            x: position.x,
                            y: position.y,
                        });
                        state.pet_position_revision.fetch_add(1, Ordering::Relaxed) + 1
                    };
                    thread::spawn(move || {
                        thread::sleep(Duration::from_millis(350));
                        if let Some(state) = app.try_state::<AppState>() {
                            if state.pet_position_revision.load(Ordering::Relaxed) == revision {
                                let settings = state.settings.lock();
                                let _ = settings::persist_settings(&state.settings_path, &settings);
                            }
                        }
                    });
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            settings::load_settings,
            settings::save_prompt_settings,
            settings::fetch_models,
            settings::default_prompt,
            pet::select_pet_appearance,
            pet::add_pet_assets,
            pet::rename_pet_asset,
            pet::delete_pet_asset,
            settings::save_shortcuts,
            shortcuts::get_shortcut_conflicts,
            settings::save_preferences,
            tracker::get_previous_app,
            tracker::list_capturable_windows,
            recording::get_recording_status,
            recording::toggle_recording,
            polish::polish_text,
            snapshots::list_snapshots,
            snapshots::open_snapshots_dir,
            recording::open_recordings_dir,
            snapshots::delete_snapshots,
            snapshots::copy_snapshot,
            activity::list_activity,
            clipboard::get_clipboard_state,
            clipboard::clear_clipboard_now,
            clipboard::keep_clipboard,
            clipboard::copy_text,
            clipboard::read_clipboard_text,
            capabilities::platform_capabilities,
            actions::show_quick_menu,
            actions::resize_quick_menu,
            actions::hide_quick_menu,
            actions::run_action,
            actions::show_main_window,
            launchpad::show_launchpad,
            launchpad::hide_launchpad,
            launchpad::launchpad_state,
            launchpad::save_launchpad,
            launchpad::launch_app,
            capture::get_annotate_image,
            capture::annotate_get_title,
            capture::annotate_copy,
            capture::annotate_save,
            capture::annotate_close,
        ])
        .run(context)
        .expect("运行 snapshot 失败");
}

#[cfg(test)]
mod pet_asset_tests {
    use super::*;

    /// 老配置含已废弃的录制目录绑定与（Win/mac 上）不支持的滚动长截图：
    /// 移除它们，补出新动作，保留其它已有绑定。
    #[test]
    fn old_settings_migrate_shortcut_actions() {
        let path = std::env::temp_dir().join("snapshot-settings-migration.json");
        let legacy = r#"{
            "baseUrl": "https://api.example.com/v1",
            "model": "demo",
            "templates": [{"id":"builtin-default","name":"内置","content":"x","builtin":true}],
            "activeTemplateId": "builtin-default",
            "selectedAppearanceId": "app-icon",
            "petAssets": [],
            "saveDir": "~/LegacyCaptures",
            "shortcuts": [
                {"action":"snapshot","accelerator":"Alt+Shift+2"},
                {"action":"record","accelerator":null},
                {"action":"recordings","accelerator":"Alt+Shift+R"},
                {"action":"polish","accelerator":null},
                {"action":"scrolling","accelerator":null}
            ]
        }"#;
        fs::write(&path, legacy).expect("写测试配置失败");

        let settings = settings::read_settings(&path);
        let actions: Vec<&str> = settings
            .shortcuts
            .iter()
            .map(|item| item.action.as_str())
            .collect();
        // 平台不支持的动作（如 Win/mac 的 scrolling）不应被迁移补回
        #[cfg(target_os = "linux")]
        assert_eq!(
            actions,
            vec![
                "snapshot",
                "record",
                "polish",
                "scrolling",
                "fullscreen",
                "palette"
            ]
        );
        #[cfg(not(target_os = "linux"))]
        assert_eq!(
            actions,
            vec!["snapshot", "record", "polish", "fullscreen", "palette"]
        );

        // 已绑定的键不能在迁移中丢失
        let snapshot = settings
            .shortcuts
            .iter()
            .find(|item| item.action == "snapshot")
            .unwrap();
        assert_eq!(snapshot.accelerator.as_deref(), Some("Alt+Shift+2"));
        // 新补的那条应当是未绑定状态
        let fullscreen = settings
            .shortcuts
            .iter()
            .find(|item| item.action == "fullscreen")
            .unwrap();
        assert!(fullscreen.accelerator.is_none());
        // 老版录制与快照共用 saveDir，新版应将它迁移到独立录制目录。
        assert_eq!(settings.recording_dir, "~/LegacyCaptures");

        let _ = fs::remove_file(&path);
    }
}
