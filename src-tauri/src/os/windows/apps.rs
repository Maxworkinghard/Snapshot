//! 启动台的应用清单，来自两处：
//!
//! - 开始菜单里的快捷方式：桌面程序。不扫注册表里的卸载项，那里混着运行库和更新包。
//! - 系统「所有应用」（shell:AppsFolder）里的打包应用：商店应用在开始菜单目录里没有快捷方式，
//!   只能从这里拿。桌面程序前面已经有了，这里只取打包的（AUMID 形如 `包族名!应用`）。
//!
//! 网格里只摆用户装的软件、商店应用和明确的桌面应用。Windows 自带的系统管理工具、辅助功能、
//! 控制面板项、诊断工具，驱动带的工具，配置程序和后台组件不进网格，但留在清单里，搜索能找到。
//! 卸载程序也只在搜索中出现；文档网址和控制台程序（cmd / PowerShell / node / python）不收。
//!
//! 能看结构就不看名字（名字是本地化的，也会被用户改）：快捷方式真正的目标（走 IShellLink 解析，
//! 和资源管理器同一条路径）、目标是不是装在 Windows 目录、PE 头里的子系统、打包应用的发布者。
//! 卸载程序和配置程序没有别的特征，只能看名字。目标没解析出来（COM 一时不可用、快捷方式损坏）
//! 一律当普通应用留在网格里，免得一次解析失败就把整份清单清空。

use std::{
    collections::HashSet,
    fs,
    fs::File,
    io::{Read, Seek, SeekFrom},
    mem::{size_of, zeroed},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::null,
};

use windows::{
    core::{Interface, PCWSTR},
    Win32::{
        Storage::EnhancedStorage::PKEY_AppUserModel_ID,
        System::Com::{
            CoCreateInstance, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ,
        },
        UI::Shell::{
            BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, IShellItem2,
            IShellLinkW, SHCreateItemFromParsingName, SHGetKnownFolderItem, ShellLink,
            KF_FLAG_DEFAULT, SIGDN, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
        },
    },
};
use windows_sys::Win32::{
    System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
    UI::{
        Shell::{SHGetFileInfoW, ShellExecuteW, SHFILEINFOW, SHGFI_DISPLAYNAME},
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

use super::icon;
use crate::launchpad::{stable_id, InstalledApp};

/// PE 可选头里的 Subsystem：3 是控制台程序，2 是图形程序。
const SUBSYSTEM_WINDOWS_GUI: u16 = 2;
const SUBSYSTEM_WINDOWS_CUI: u16 = 3;

/// 打包应用的启动目标写成 `shell:AppsFolder\AUMID`，ShellExecute 和 shell 取图标都认。
pub(crate) const APPS_FOLDER: &str = "shell:AppsFolder\\";

/// 发布者是「Microsoft Windows」的包：设置、Windows 备份、入门、单击以执行这些系统自带的部分。
const WINDOWS_PUBLISHER: &str = "cw5n1h2txyewy";

/// 从商店更新、其实是系统工具的包：安全中心、反馈中心、获取帮助、快速助手、Game Bar 叠加层。
const SYSTEM_PACKAGES: &[&str] = &[
    "microsoft.sechealthui",
    "microsoft.windowsfeedbackhub",
    "microsoft.gethelp",
    "microsoftcorporationii.quickassist",
    "microsoft.xboxgamingoverlay",
];

/// 硬件厂商随驱动装的控制台（显卡、声卡、触控板）。打包应用看包名开头。
const HARDWARE_PACKAGES: &[&str] = &[
    "advancedmicrodevices",
    "nvidiacorp",
    "appup.intel",
    "intelcorp",
    "realteksemiconductor",
    "synaptics",
    "elanmicroelectronics",
    "dolbylaboratories",
    "dtsinc",
    "wavesaudio",
    "conexant",
    "a-volute",
    "qualcomm",
    "mediatek",
];

/// 同上，桌面程序看装在 Program Files 下哪个目录。
const HARDWARE_FOLDERS: &[&str] = &[
    "amd",
    "ati technologies",
    "nvidia corporation",
    "intel",
    "realtek",
    "synaptics",
    "elantech",
    "dolby",
    "waves",
    "conexant",
    "nahimic",
    "qualcomm",
    "mediatek",
];

/// 配置程序的名字里会出现的词。英文按整词比，中文按字串比。
const SETUP_WORDS: &[&str] = &[
    "config",
    "configuration",
    "configure",
    "configurator",
    "settings",
    "setup",
    "preferences",
];
const SETUP_PHRASES: &[&str] = &["配置", "设置", "設定", "設置", "偏好", "喜好"];

/// 在启动台里放在哪。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shelf {
    /// 摆进网格：用户装的软件、商店应用、明确的桌面应用
    Grid,
    /// 不进网格，搜索时才出现：系统管理工具、辅助功能、控制面板项、诊断、驱动工具、配置程序、后台组件
    Search,
    /// 不收：文档网址、控制台程序
    Skip,
}

/// 候选应用及用于去重的启动身份（目标与参数；None 表示没解析出来）。
struct Candidate {
    app: InstalledApp,
    target: Option<String>,
}

pub(crate) fn installed_apps() -> Vec<InstalledApp> {
    let _com = ComApartment::enter();
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for root in roots() {
        collect(&root, &mut found, &mut seen, 0);
    }
    found.sort_by(|left, right| {
        right
            .app
            .listed
            .cmp(&left.app.listed)
            .then_with(|| by_name(&left.app, &right.app))
    });

    // 同一个程序在开始菜单里可能有多个快捷方式（比如 Tailscale 顶层和文件夹里各一个），
    // 按解析出来的目标去重，留下排序靠前的那个。
    let mut targets = HashSet::new();
    let mut apps = Vec::with_capacity(found.len());
    for candidate in found {
        if let Some(target) = candidate.target.as_deref() {
            let target = target.trim();
            if !target.is_empty() && !targets.insert(target.to_string()) {
                continue;
            }
        }
        apps.push(candidate.app);
    }

    // 有些打包应用也放了快捷方式，按 AUMID 去重并保留原 id；同名但不同的应用不能互相遮掉。
    apps.extend(
        packaged_apps()
            .into_iter()
            .filter(|app| targets.insert(normalize(Path::new(&app.target)))),
    );
    apps.sort_by(by_name);
    apps
}

fn by_name(left: &InstalledApp, right: &InstalledApp) -> std::cmp::Ordering {
    left.name
        .to_lowercase()
        .cmp(&right.name.to_lowercase())
        .then(left.id.cmp(&right.id))
}

pub(crate) fn launch_target(target: &str) -> Result<(), String> {
    let _com = ComApartment::enter();
    let operation = wide("open");
    let file = wide(target);
    let code = unsafe {
        // 快捷方式要走 Shell，公寓线程没初始化时有的机器会直接失败
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
        if stem.is_empty() {
            continue;
        }
        let id = stable_id(&normalize(&path));
        if !seen.insert(id.clone()) {
            continue;
        }
        // 资源管理器里显示的名字：Windows 自带的快捷方式文件名是英文，本地化的名字在 desktop.ini 里
        let name = display_name(&path).unwrap_or_else(|| stem.clone());
        let shortcut = shortcut_target(&path);
        let target = shortcut_aumid(&path)
            .map(|aumid| format!("{APPS_FOLDER}{aumid}"))
            .or_else(|| {
                shortcut.as_ref().and_then(|(target, arguments)| {
                    let arguments = arguments.trim().trim_matches('"');
                    (Path::new(target)
                        .file_name()
                        .is_some_and(|file| file.eq_ignore_ascii_case("explorer.exe"))
                        && arguments.starts_with(APPS_FOLDER))
                    .then(|| arguments.to_string())
                })
            })
            .or_else(|| shortcut.as_ref().map(|(target, _)| target.clone()));
        let arguments = shortcut
            .as_ref()
            .map(|(_, args)| args.as_str())
            .unwrap_or("");
        let shelf = shortcut_shelf(&format!("{stem} {name}"), target.as_deref(), arguments);
        if shelf == Shelf::Skip {
            continue;
        }
        into.push(Candidate {
            app: InstalledApp {
                id,
                name: crate::truncate(&name, 80),
                target: path.to_string_lossy().to_string(),
                listed: shelf == Shelf::Grid,
            },
            target: target.map(|target| shortcut_identity(&target, arguments)),
        });
    }
}

fn normalize(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

/// 快捷方式放在哪。`label` 是文件名加显示名，只有卸载程序、安装器、配置程序要看它。
/// target 为 None 表示没解析出来，按未知处理，留在网格里。
fn shortcut_shelf(label: &str, target: Option<&str>, arguments: &str) -> Shelf {
    if uninstaller(label, target) {
        return Shelf::Search;
    }
    if setup_tool(label) {
        return Shelf::Search;
    }
    let Some(target) = target.map(str::trim) else {
        return Shelf::Grid;
    };
    if let Some(aumid) = target.strip_prefix(APPS_FOLDER) {
        return packaged_shelf(aumid);
    }
    if target.is_empty() {
        // 控制面板、运行、Windows 工具这类 shell 项，没有文件目标
        return Shelf::Search;
    }
    let path = Path::new(target);
    if path
        .file_name()
        .is_some_and(|file| file.eq_ignore_ascii_case("explorer.exe"))
        && !arguments.trim().is_empty()
    {
        // explorer.exe 也用来打开 SDK 文件夹或打包应用；有 .exe 入口不代表它本身是应用。
        let argument = arguments.trim().trim_matches('"');
        if let Some(aumid) = argument.strip_prefix(APPS_FOLDER) {
            return packaged_shelf(aumid);
        }
        return if windows_component(Path::new(argument)) {
            Shelf::Search
        } else {
            Shelf::Skip
        };
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "exe" => {}
        // 事件查看器、服务这类管理单元，控制面板小程序
        "msc" | "cpl" => return Shelf::Search,
        // 帮助文档、网址、许可证
        _ => return Shelf::Skip,
    }
    // 只有读出来确实是控制台程序才不收；读不出来（文件没了、不是 PE）当未知
    if subsystem(path) == Some(SUBSYSTEM_WINDOWS_CUI) {
        return Shelf::Skip;
    }
    if windows_component(path) || driver_tool(path) || installer(label, path) {
        return Shelf::Search;
    }
    Shelf::Grid
}

/// 卸载程序：名字里带「卸载」，或者目标本身就是卸载程序（unins000.exe、uninst.exe、Uninstall.exe）。
fn uninstaller(label: &str, target: Option<&str>) -> bool {
    let label = label.to_lowercase();
    if label.contains("uninstall") || label.contains("卸载") {
        return true;
    }
    let file = target
        .and_then(|target| Path::new(target.trim()).file_stem())
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_lowercase();
    file.starts_with("unins") || file.contains("卸载")
}

/// 安装器和更新器：名字里有 Installer，或者目标可执行文件本身就是更新程序。
fn installer(label: &str, target: &Path) -> bool {
    let label = label.to_lowercase();
    if words(&label).any(|word| word == "installer") || label.contains("安装程序") {
        return true;
    }
    let file = target
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_lowercase();
    file.ends_with("update")
        || file.ends_with("updater")
        || file.ends_with("setup")
        || file.ends_with("installer")
}

/// 配置程序：没有结构上的特征（WPS「配置工具」的可执行文件只叫 ksomisc.exe），只能看名字。
fn setup_tool(label: &str) -> bool {
    let label = label.to_lowercase();
    words(&label).any(|word| SETUP_WORDS.contains(&word))
        || SETUP_PHRASES.iter().any(|phrase| label.contains(phrase))
}

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|letter: char| !letter.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
}

/// Windows 目录里的系统组件，保留明确的桌面应用；Windows Kits 等辅助目录也只在搜索中出现。
fn windows_component(target: &Path) -> bool {
    let target = normalize(target);
    let windows = std::env::var_os("SystemRoot").or_else(|| std::env::var_os("windir"));
    if let Some(windows) = windows {
        let root = format!(
            "{}\\",
            normalize(Path::new(&windows)).trim_end_matches('\\')
        );
        if let Some(relative) = target.strip_prefix(&root) {
            return !matches!(
                relative,
                "notepad.exe"
                    | "explorer.exe"
                    | "system32\\notepad.exe"
                    | "system32\\mspaint.exe"
                    | "system32\\calc.exe"
                    | "system32\\snippingtool.exe"
            );
        }
    }
    program_files_folder(&target).is_some_and(|folder| {
        matches!(
            folder.as_str(),
            "windows kits" | "windows defender" | "windows nt" | "windows mail"
        )
    })
}

/// 硬件厂商随驱动装的控制台。装在驱动库里的已经算 Windows 目录。
fn driver_tool(target: &Path) -> bool {
    program_files_folder(&normalize(target))
        .is_some_and(|folder| HARDWARE_FOLDERS.contains(&folder.as_str()))
}

/// 目标装在 Program Files（64 / 32 位）下时，返回它下面的第一层目录名（小写）。
fn program_files_folder(target: &str) -> Option<String> {
    ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
        .into_iter()
        .filter_map(std::env::var_os)
        .find_map(|root| {
            let root = format!("{}\\", normalize(Path::new(&root)).trim_end_matches('\\'));
            let rest = target.strip_prefix(&root)?;
            rest.split('\\').next().map(str::to_string)
        })
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

/// 资源管理器里显示的名字。拿不到时调用方退回文件名。
fn display_name(link: &Path) -> Option<String> {
    let path: Vec<u16> = link
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut info: SHFILEINFOW = unsafe { zeroed() };
    let found = unsafe {
        SHGetFileInfoW(
            path.as_ptr(),
            0,
            &mut info,
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_DISPLAYNAME,
        )
    };
    if found == 0 {
        return None;
    }
    let len = info
        .szDisplayName
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(info.szDisplayName.len());
    let name = String::from_utf16_lossy(&info.szDisplayName[..len]);
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// 打包应用可能也有 .lnk。读取身份再分类和去重，不能把它的空文件目标当成控制面板项。
fn shortcut_aumid(link: &Path) -> Option<String> {
    unsafe {
        let path: Vec<u16> = link
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let item: IShellItem2 = SHCreateItemFromParsingName(PCWSTR(path.as_ptr()), None).ok()?;
        let raw = item.GetString(&PKEY_AppUserModel_ID).ok()?;
        let text = raw.to_string().ok();
        CoTaskMemFree(Some(raw.0 as *const _));
        text.filter(|id| id.contains('!') || id.eq_ignore_ascii_case("Microsoft.Windows.Explorer"))
    }
}

/// 快捷方式的真实目标。走 IShellLink 解析，和资源管理器同一条路径：
/// 命令行、事件查看器这类快捷键没有 LinkInfo，光解析文件格式会漏掉。
fn shortcut_target(link: &Path) -> Option<(String, String)> {
    let _com = ComApartment::enter();
    unsafe {
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
        let mut arguments = [0u16; 4096];
        let _ = shell_link.GetArguments(&mut arguments);
        let len = arguments
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(arguments.len());
        Some((expand(&text), String::from_utf16_lossy(&arguments[..len])))
    }
}

fn shortcut_identity(target: &str, arguments: &str) -> String {
    let target = normalize(Path::new(target));
    if target.starts_with(&APPS_FOLDER.to_ascii_lowercase())
        || arguments.trim().is_empty()
        || target.trim().is_empty()
    {
        return target;
    }
    format!("{target}\0{}", arguments.trim())
}

/// 系统「所有应用」里的打包应用。拿不到（COM 不可用、精简系统没有这个文件夹）就是空的，
/// 不影响开始菜单那一份。
fn packaged_apps() -> Vec<InstalledApp> {
    let _com = ComApartment::enter();
    let mut apps = Vec::new();
    unsafe {
        let Ok(folder) =
            SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)
        else {
            return apps;
        };
        let Ok(entries) = folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else {
            return apps;
        };
        loop {
            let mut batch = [None];
            let mut fetched = 0u32;
            if entries.Next(&mut batch, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(item) = batch[0].take() else {
                break;
            };
            // 桌面程序在开始菜单那份里已经有了，这里只要打包应用
            let Some(aumid) = shell_name(&item, SIGDN_PARENTRELATIVEPARSING) else {
                continue;
            };
            if !aumid.contains('!') {
                continue;
            }
            let name = shell_name(&item, SIGDN_NORMALDISPLAY).unwrap_or_else(|| aumid.clone());
            let target = format!("{APPS_FOLDER}{aumid}");
            apps.push(InstalledApp {
                id: stable_id(&target.to_lowercase()),
                name: crate::truncate(&name, 80),
                target,
                listed: !setup_tool(&name)
                    && !uninstaller(&name, None)
                    && packaged_shelf(&aumid) == Shelf::Grid,
            });
        }
    }
    apps
}

/// 打包应用放在哪：Windows 自己的包、系统工具、驱动带的控制台和疑难解答只在搜索里。
fn packaged_shelf(aumid: &str) -> Shelf {
    let aumid = aumid.to_lowercase();
    let Some((family, entry)) = aumid.split_once('!') else {
        return Shelf::Grid;
    };
    let (package, publisher) = family.rsplit_once('_').unwrap_or((family, ""));
    if publisher == WINDOWS_PUBLISHER
        || SYSTEM_PACKAGES.contains(&package)
        || HARDWARE_PACKAGES
            .iter()
            .any(|vendor| package.starts_with(vendor))
        || entry.contains("troubleshoot")
    {
        return Shelf::Search;
    }
    Shelf::Grid
}

unsafe fn shell_name(item: &IShellItem, kind: SIGDN) -> Option<String> {
    let raw = item.GetDisplayName(kind).ok()?;
    let text = raw.to_string().ok();
    CoTaskMemFree(Some(raw.0 as *const _));
    text.map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
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

/// 每次成功初始化都配对释放，避免打开启动台时给线程不断累积 COM 引用。
struct ComApartment(bool);

impl ComApartment {
    fn enter() -> Self {
        Self(unsafe { CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32) } >= 0)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_apps_have_targets_and_no_duplicates() {
        let apps = installed_apps();
        assert!(!apps.is_empty(), "开始菜单里应该能找到快捷方式");
        assert!(apps.iter().any(|app| app.listed), "网格里不该是空的");
        let mut ids = HashSet::new();
        for app in &apps {
            assert!(!app.name.trim().is_empty());
            assert!(
                app.target.to_ascii_lowercase().ends_with(".lnk")
                    || app.target.starts_with(APPS_FOLDER),
                "{}",
                app.target
            );
            assert!(ids.insert(app.id.clone()), "重复的应用：{}", app.name);
        }
    }

    #[test]
    fn most_apps_have_png_icons() {
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
            "只有 {ok}/{} 个应用取出了图标，例如 {}",
            apps.len(),
            missing.join("、")
        );
    }

    #[test]
    fn packaged_icons_are_visible_and_match_the_requested_size() {
        for app in packaged_apps() {
            for size in [32, 96] {
                let png = launch_icon_png(&app.target, size)
                    .unwrap_or_else(|| panic!("{} 缺少 {size}px 商店图标", app.name));
                let image = image::load_from_memory(&png).unwrap().into_rgba8();
                assert_eq!(image.dimensions(), (size, size), "{}", app.name);
                assert!(
                    image.pixels().any(|pixel| pixel[3] > 0),
                    "{} 图标完全透明",
                    app.name
                );
            }
        }
    }

    /// 同一份名单里不该出现同一个可执行文件两次（Tailscale 有两个快捷方式）。
    #[test]
    fn no_target_appears_twice() {
        let apps = installed_apps();
        let mut targets = HashSet::new();
        for app in &apps {
            if let Some((target, arguments)) = shortcut_target(Path::new(&app.target)) {
                let target = target.trim();
                if target.is_empty() {
                    continue;
                }
                assert!(
                    targets.insert(shortcut_identity(target, &arguments)),
                    "{} 出现了两次",
                    app.name
                );
            }
        }
    }

    /// 网格里是用户装的软件；系统工具、驱动工具、配置程序、安装器只在搜索里；
    /// 卸载程序仅搜索；文档、控制台程序不收；读不出来的目标按未知留在网格里。
    #[test]
    fn shortcuts_land_on_the_right_shelf() {
        use Shelf::{Grid, Search, Skip};
        let windows = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let program_files =
            std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
        let system = |file: &str| format!(r"{windows}\System32\{file}");
        let installed = |rest: &str| format!(r"{program_files}\{rest}");

        let cases = [
            (
                "Blender 5.2",
                Some(installed(r"Blender\blender-launcher.exe")),
                Grid,
            ),
            ("File Explorer", None, Grid),
            (
                "文件资源管理器",
                Some(format!("{APPS_FOLDER}Microsoft.Windows.Explorer")),
                Grid,
            ),
            (
                "Windows Software Development Kit",
                Some(r"C:\ProgramData\Package Cache\sdk\winsdksetup.exe".to_string()),
                Search,
            ),
            (
                "Registry Editor 注册表编辑器",
                Some(format!(r"{windows}\regedit.exe")),
                Search,
            ),
            ("Magnify 放大镜", Some(system("magnify.exe")), Search),
            ("Event Viewer", Some(system("eventvwr.msc")), Search),
            ("Control Panel 控制面板", Some(String::new()), Search),
            (
                "Windows Media Player Legacy",
                Some(installed(r"Windows Media Player\wmplayer.exe")),
                Grid,
            ),
            ("Notepad 记事本", Some(system("notepad.exe")), Grid),
            ("Paint 画图", Some(system("mspaint.exe")), Grid),
            (
                "GPUView",
                Some(installed(
                    r"Windows Kits\10\Windows Performance Toolkit\gpuview\GPUView.exe",
                )),
                Search,
            ),
            (
                "AMD Software",
                Some(installed(r"AMD\CNext\CNext\RadeonSoftware.exe")),
                Search,
            ),
            (
                "配置工具",
                Some(installed(r"Kingsoft\WPS Office\office6\ksomisc.exe")),
                Search,
            ),
            (
                "Office 語言喜好設定",
                Some(installed(r"Microsoft Office\root\Office16\SETLANG.EXE")),
                Search,
            ),
            (
                "Visual Studio Installer",
                Some(r"C:\VS\setup.exe".to_string()),
                Search,
            ),
            (
                "微信输入法",
                Some(r"C:\Tencent\WeType\wetype_update.exe".to_string()),
                Search,
            ),
            (
                "卸载微信",
                Some(installed(r"Tencent\Weixin\Uninstall.exe")),
                Search,
            ),
            (
                "Uninstall Dev-C++",
                Some(installed(r"Dev-Cpp\uninstall.exe")),
                Search,
            ),
            (
                "WeGame",
                Some(r"F:\WeGame\unins000.exe".to_string()),
                Search,
            ),
            ("License", Some(installed(r"Dev-Cpp\COPYING.txt")), Skip),
            ("Command Prompt 命令提示符", Some(system("cmd.exe")), Skip),
        ];
        for (label, target, expected) in cases {
            assert_eq!(
                shortcut_shelf(label, target.as_deref(), ""),
                expected,
                "{label}"
            );
        }
    }

    #[test]
    fn explorer_wrappers_are_classified_by_what_they_open() {
        let windows = std::env::var("SystemRoot").unwrap();
        let explorer = format!(r"{windows}\explorer.exe");
        let program_files = std::env::var("ProgramFiles").unwrap();
        let sdk = format!(r#""{program_files}\Windows Kits\10\""#);
        assert_eq!(
            shortcut_shelf("Windows Software Development Kit", Some(&explorer), &sdk),
            Shelf::Search
        );
        assert_eq!(shortcut_shelf("Files", Some(&explorer), ""), Shelf::Grid);
        assert_eq!(
            shortcut_shelf("Documents", Some(&explorer), r"C:\Users\Public\Documents"),
            Shelf::Skip
        );
        assert_eq!(
            shortcut_shelf(
                "Calculator",
                Some(&explorer),
                &format!("{APPS_FOLDER}Microsoft.WindowsCalculator_8wekyb3d8bbwe!App")
            ),
            Shelf::Grid
        );
        assert_ne!(
            shortcut_identity(&explorer, "app-a"),
            shortcut_identity(&explorer, "app-b")
        );
    }

    /// 商店应用进网格；Windows 自己的包、系统工具、驱动带的控制台、疑难解答只在搜索里。
    #[test]
    fn packaged_apps_land_on_the_right_shelf() {
        use Shelf::{Grid, Search};
        let cases = [
            ("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App", Grid),
            ("Microsoft.WindowsTerminal_8wekyb3d8bbwe!App", Grid),
            ("Claude_pzs8sxrjxfjjc!Claude", Grid),
            ("OpenAI.Codex_2p2nqsd0c76g0!App", Grid),
            ("Microsoft.PowerAutomateDesktop_8wekyb3d8bbwe!PAD.Console", Grid),
            ("windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel", Search),
            ("MicrosoftWindows.Client.CBS_cw5n1h2txyewy!WindowsBackup", Search),
            ("Microsoft.SecHealthUI_8wekyb3d8bbwe!SecHealthUI", Search),
            ("Microsoft.WindowsFeedbackHub_8wekyb3d8bbwe!App", Search),
            ("Microsoft.XboxGamingOverlay_8wekyb3d8bbwe!App", Search),
            ("Microsoft.PowerAutomateDesktop_8wekyb3d8bbwe!PAD.Troubleshooter", Search),
            ("AdvancedMicroDevicesInc-2.AMDRadeonSoftware_0a9344xs7nr4m!AMDRadeonsoftwareUWP", Search),
        ];
        for (aumid, expected) in cases {
            assert_eq!(packaged_shelf(aumid), expected, "{aumid}");
        }
    }
}
