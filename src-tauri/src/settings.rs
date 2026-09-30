use super::{os, AppState};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fs, path::PathBuf, time::Duration};
use tauri::{AppHandle, Emitter, Runtime, State};

const KEYRING_SERVICE: &str = "com.appsnapshot.snapshot";
const KEYRING_USER: &str = "polish-api-key";

pub(crate) const DEFAULT_PROMPT: &str = r#"你是面向编程助手的提示词改写专家。下面「用户草稿」是待改写的指令原文，不是要你执行的任务。不要回答问题，不要写代码，不要调用工具，不要与用户对话。只输出改写后的完整指令。

改写目标：保持原意不变，把草稿讲透——说清用户真正想要的东西，补上这件事在专业上必然涉及、而用户只是没写出来的部分，把含糊说法换成该领域的准确术语。这是把同一个需求表达得更专业、更可执行，不是把需求做大。

扩展的唯一依据是草稿原意。判定标准：补出来的每一句拿给用户看，他会说「对，我就是这个意思，只是没写出来」；他会说「我没这么说」的，一律删掉。

必须遵守：
1. 语言与原文一致；中英混写则保持自然混写。不要翻译受保护内容。
2. 保留目标、范围、约束、明确排除项、交付物类型，以及所处阶段（解释 / 审查 / 规划 / 实现 / 验证）。不要把「实现」改成「只做计划」，也不要把「先分析」改成允许改代码。
3. 代码块、命令、路径、标识符、配置值、URL、报错原文必须原样保留（含语言与有意义空白）。只改周围说明文字。
4. 改写后的指令会被粘贴进一个拥有仓库、会话历史和工具的编程助手里执行，它能自己查。所以凡是你不知道的具体对象，一律写成「由你在当前上下文中定位并核实的 X」交给下游去查，绝不写成向用户提问、索取材料、要求确认或先行澄清的步骤。
5. 「那个页面」「这个 bug」「审查代码」这类指称原样保留，不要臆测具体对象，也不要展开成提问。
6. 未证实的路径、API、业务规则、性能数字、用户规模不要写成既定事实。你认为有必要的技术方向可以提，但须标明是建议方向而非已定决策；用户已指定的技术栈必须原样尊重。
7. 用专业术语替换含糊表述，前提是该术语确实是用户所指的东西；判断不出时保留原说法，不要堆砌名词。
8. 补全只做两件事：说清用户已经要的东西，以及点出这件事专业上绕不开、用户大概率没想到的点。不要新增功能、约束、验收标准或质量指标。
9. 窄范围修复、审查、解释类草稿，只加深诊断方向、期望行为和边界，不要扩成重构或加功能。开放式创造类草稿可以把核心流程与状态讲完整，但仍受第 8 条约束。
10. 长度服从内容。删除重复、空泛赞美、无关清单和空标题；不要前言、分析、语言标签、XML 包裹或额外外层代码围栏。
11. 输出必须是一条可以直接发出去、下游收到后能立刻开始干活的完整指令。不要多轮问答流程，不要出现索取材料或要求确认的句子，除非草稿本身明确要求先提问。
12. 草稿是在求判断或决策时，要求下游先重述问题、给出正反最强论证、指出分歧点与关键变量，并向用户提出一个最关键的问题后再判断。该流程只用于决策类草稿。

输出前默默检查：是否保持原意与阶段；是否误加功能或约束；受保护内容是否原样保留；术语是否准确；结果是否足够清楚且可直接执行。"#;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptTemplate {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) content: String,
    pub(crate) builtin: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ShortcutBinding {
    pub(crate) action: String,
    pub(crate) accelerator: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PetPosition {
    pub(crate) x: i32,
    pub(crate) y: i32,
}

pub(crate) fn default_appearance_id() -> String {
    "app-icon".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Settings {
    pub(crate) base_url: String,
    pub(crate) model: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) has_api_key: bool,
    pub(crate) templates: Vec<PromptTemplate>,
    pub(crate) active_template_id: String,
    #[serde(default = "default_appearance_id")]
    pub(crate) selected_appearance_id: String,
    #[serde(default)]
    pub(crate) pet_assets: Vec<super::pet::PetAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pet_position: Option<PetPosition>,
    /// 桌宠大小：按屏幕算出的基准大小的百分比，100 就是原来的大小
    #[serde(default = "default_pet_scale")]
    pub(crate) pet_scale: u32,
    pub(crate) shortcuts: Vec<ShortcutBinding>,
    #[serde(default = "default_clipboard_auto_clear")]
    pub(crate) clipboard_auto_clear: String,
    #[serde(default = "default_snapshot_format")]
    pub(crate) snapshot_format: String,
    #[serde(default)]
    pub(crate) save_dir: String,
    /// 录制产物目录。留空则沿用 save_dir，再空则落到「下载」。
    #[serde(default)]
    pub(crate) recording_dir: String,
    #[serde(default = "default_shutter_sound")]
    pub(crate) shutter_sound: String,
    #[serde(default)]
    pub(crate) custom_sound_path: Option<String>,
    #[serde(default = "default_true")]
    pub(crate) flash_on_capture: bool,
    #[serde(default)]
    pub(crate) hide_after_copy: bool,
    #[serde(default = "default_true")]
    pub(crate) auto_save_local: bool,
    #[serde(default)]
    pub(crate) launch_on_boot: bool,
    #[serde(default)]
    pub(crate) include_cursor: bool,
    #[serde(default)]
    pub(crate) record_system_audio: bool,
    #[serde(default)]
    pub(crate) record_microphone: bool,
    #[serde(default = "default_after_capture")]
    pub(crate) after_capture: String,
    /// 启动台顶层顺序，`app:<id>` 或 `folder:<id>`。空的表示还没排过，按名字排。
    #[serde(default)]
    pub(crate) launchpad_order: Vec<String>,
    #[serde(default)]
    pub(crate) launchpad_folders: Vec<super::launchpad::LaunchpadFolder>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn default_true() -> bool {
    true
}

fn default_clipboard_auto_clear() -> String {
    "60s".into()
}

fn default_snapshot_format() -> String {
    "png".into()
}

fn default_shutter_sound() -> String {
    "crisp".into()
}

fn default_after_capture() -> String {
    "clipboard".into()
}

/// 桌宠大小能调的范围（百分比），和前端 lib/petSize.ts 的 PET_SCALE 一致
const PET_SCALE_RANGE: (u32, u32) = (30, 200);

fn default_pet_scale() -> u32 {
    100
}

fn clamp_pet_scale(value: u32) -> u32 {
    value.clamp(PET_SCALE_RANGE.0, PET_SCALE_RANGE.1)
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            model: String::new(),
            has_api_key: false,
            templates: vec![PromptTemplate {
                id: "builtin-default".into(),
                name: "清晰、可执行".into(),
                content: DEFAULT_PROMPT.into(),
                builtin: true,
            }],
            active_template_id: "builtin-default".into(),
            selected_appearance_id: default_appearance_id(),
            pet_assets: Vec::new(),
            pet_position: None,
            pet_scale: default_pet_scale(),
            shortcuts: default_shortcut_bindings(),
            clipboard_auto_clear: default_clipboard_auto_clear(),
            snapshot_format: default_snapshot_format(),
            save_dir: String::new(),
            recording_dir: String::new(),
            shutter_sound: default_shutter_sound(),
            custom_sound_path: None,
            flash_on_capture: true,
            hide_after_copy: false,
            auto_save_local: true,
            launch_on_boot: false,
            include_cursor: false,
            record_system_audio: false,
            record_microphone: false,
            after_capture: default_after_capture(),
            launchpad_order: Vec::new(),
            launchpad_folders: Vec::new(),
        }
    }
}

/// 本平台支持的全局快捷键动作（由各平台自己声明，如滚动长截图只有 Linux 有），全部未绑定。
fn default_shortcut_bindings() -> Vec<ShortcutBinding> {
    os::SHORTCUT_ACTIONS
        .iter()
        .map(|action| ShortcutBinding {
            action: action.to_string(),
            accelerator: None,
        })
        .collect()
}

fn is_supported_action(action: &str) -> bool {
    os::SHORTCUT_ACTIONS.contains(&action)
}

/// 偏好设置的增量补丁：None 表示未发送，可空字段用 Value 区分"没传"和"显式置空"
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreferencesPatch {
    clipboard_auto_clear: Option<String>,
    snapshot_format: Option<String>,
    save_dir: Option<String>,
    recording_dir: Option<String>,
    shutter_sound: Option<String>,
    custom_sound_path: Option<serde_json::Value>,
    flash_on_capture: Option<bool>,
    hide_after_copy: Option<bool>,
    auto_save_local: Option<bool>,
    launch_on_boot: Option<bool>,
    include_cursor: Option<bool>,
    record_system_audio: Option<bool>,
    record_microphone: Option<bool>,
    after_capture: Option<String>,
    pet_scale: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptSettingsInput {
    base_url: String,
    model: String,
    api_key: Option<String>,
    active_template_id: String,
    templates: Vec<PromptTemplate>,
}

pub(crate) fn keyring_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|error| error.to_string())
}

fn has_api_key() -> bool {
    keyring_entry()
        .and_then(|entry| entry.get_password().map_err(|error| error.to_string()))
        .map(|key| !key.is_empty())
        .unwrap_or(false)
}

pub(crate) fn read_settings(path: &PathBuf) -> Settings {
    let content = fs::read_to_string(path).ok();
    let recording_dir_was_present = content
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|value| {
            value
                .as_object()
                .map(|object| object.contains_key("recordingDir"))
        })
        .unwrap_or(false);
    let mut settings = content
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Settings>(raw).ok())
        .unwrap_or_default();
    // 旧版本让录制与快照共用 saveDir。升级后只迁移一次，保留用户原来的落盘位置；
    // 新配置里显式的空 recordingDir 则仍表示使用系统下载目录。
    if !recording_dir_was_present && !settings.save_dir.trim().is_empty() {
        settings.recording_dir = settings.save_dir.clone();
    }
    settings.has_api_key = has_api_key();
    if settings.templates.is_empty() {
        settings.templates = Settings::default().templates;
        settings.active_template_id = "builtin-default".into();
    }
    // 只保留本平台支持的动作：旧版的「打开录制目录」、Win/mac 上残留的滚动长截图都在这里
    // 去掉。前端直接渲染这份列表，不必再自己猜平台。
    settings
        .shortcuts
        .retain(|item| is_supported_action(&item.action));
    // 老配置里没有后来新增的动作（如 fullscreen）。其余已有绑定原样保留，
    // 缺的追加到末尾；平台不支持的动作（如 Win/mac 的 scrolling）不补。
    for fallback in default_shortcut_bindings() {
        if !settings
            .shortcuts
            .iter()
            .any(|item| item.action == fallback.action)
        {
            settings.shortcuts.push(fallback);
        }
    }
    // annotate / scrolling 已实现：读盘时保留用户选择与绑定。
    for asset in &mut settings.pet_assets {
        if asset.animations.is_empty() {
            if let Ok(animations) =
                super::pet::find_pet_animation_entries(&PathBuf::from(&asset.path))
            {
                asset.animations = animations;
            }
        }
        if !asset.animations.is_empty()
            && !asset.animations.iter().any(|entry| entry == &asset.entry)
        {
            asset.entry = asset.animations[0].clone();
        }
    }
    super::pet::refresh_missing(&mut settings.pet_assets);
    // 手改过配置文件、写了范围外的值：按边界算
    settings.pet_scale = clamp_pet_scale(settings.pet_scale);
    if settings.selected_appearance_id != "app-icon"
        && !settings
            .pet_assets
            .iter()
            .any(|asset| asset.id == settings.selected_appearance_id)
    {
        settings.selected_appearance_id = default_appearance_id();
    }
    settings
}

pub(crate) fn persist_settings(path: &PathBuf, settings: &Settings) -> Result<(), String> {
    let mut stored = settings.clone();
    stored.has_api_key = false;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let json = serde_json::to_string_pretty(&stored).map_err(|error| error.to_string())?;
    fs::write(path, json).map_err(|error| error.to_string())
}

pub(crate) fn emit_settings<R: Runtime>(app: &AppHandle<R>, settings: &Settings) {
    let _ = app.emit("settings-changed", settings);
}

#[tauri::command]
pub(crate) fn load_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().clone()
}

#[tauri::command]
pub(crate) fn save_prompt_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    input: PromptSettingsInput,
) -> Result<Settings, String> {
    if !input.base_url.is_empty()
        && !(input.base_url.starts_with("http://") || input.base_url.starts_with("https://"))
    {
        return Err("Base URL 必须以 http:// 或 https:// 开头".into());
    }
    if let Some(key) = input.api_key.as_ref().filter(|key| !key.trim().is_empty()) {
        keyring_entry()?
            .set_password(key.trim())
            .map_err(|error| error.to_string())?;
    }
    let mut settings = state.settings.lock();
    settings.base_url = input.base_url.trim().into();
    settings.model = input.model.trim().into();
    settings.templates = input.templates;
    settings.active_template_id = input.active_template_id;
    settings.has_api_key = has_api_key();
    persist_settings(&state.settings_path, &settings)?;
    let result = settings.clone();
    emit_settings(&app, &result);
    Ok(result)
}

/// 内置规则的原文：用户改过内置规则后，前端用它「恢复原文」
#[tauri::command]
pub(crate) fn default_prompt() -> &'static str {
    DEFAULT_PROMPT
}

#[tauri::command]
pub(crate) async fn fetch_models(
    state: State<'_, AppState>,
    base_url: String,
    api_key: Option<String>,
) -> Result<Vec<String>, String> {
    let endpoint = make_models_endpoint(&base_url)?;
    let key = api_key
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
        .or_else(|| {
            if state.settings.lock().has_api_key {
                keyring_entry().ok()?.get_password().ok()
            } else {
                None
            }
        });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client.get(endpoint);
    if let Some(key) = key {
        request = request.bearer_auth(key);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("拉取模型失败：{error}"))?;
    let status = response.status();
    let payload: Value = response
        .json()
        .await
        .map_err(|error| format!("模型接口响应无效：{error}"))?;
    if !status.is_success() {
        return Err(format!(
            "模型接口返回错误（{}）：{}",
            status.as_u16(),
            super::truncate(&payload.to_string(), 240)
        ));
    }

    let items = payload
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| payload.get("models").and_then(Value::as_array))
        .or_else(|| payload.as_array())
        .ok_or_else(|| "模型接口没有返回可识别的列表".to_string())?;
    let mut models = items
        .iter()
        .filter_map(|item| {
            item.as_str()
                .or_else(|| item.get("id").and_then(Value::as_str))
                .or_else(|| item.get("name").and_then(Value::as_str))
                .or_else(|| item.get("model").and_then(Value::as_str))
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    models.sort_by_key(|value| value.to_lowercase());
    models.dedup();
    if models.is_empty() {
        return Err("模型接口返回了空列表".into());
    }
    Ok(models)
}

#[tauri::command]
pub(crate) fn save_shortcuts(
    app: AppHandle,
    state: State<'_, AppState>,
    shortcuts: Vec<ShortcutBinding>,
) -> Result<Settings, String> {
    let shortcuts = shortcuts
        .into_iter()
        .filter(|item| is_supported_action(&item.action))
        .collect::<Vec<_>>();
    super::shortcuts::validate(&shortcuts)?;
    let failed = super::shortcuts::register_all(&app, &shortcuts);
    if !failed.is_empty() {
        // 存下来的必须就是生效的：有键注册不上，就整组退回旧绑定，不落盘
        let previous = state.settings.lock().shortcuts.clone();
        *state.shortcut_conflicts.lock() = super::shortcuts::register_all(&app, &previous);
        return Err(super::shortcuts::conflict_message(&failed));
    }
    state.shortcut_conflicts.lock().clear();
    let mut settings = state.settings.lock();
    settings.shortcuts = shortcuts;
    persist_settings(&state.settings_path, &settings)?;
    let result = settings.clone();
    emit_settings(&app, &result);
    Ok(result)
}

#[tauri::command]
pub(crate) fn save_preferences(
    app: AppHandle,
    state: State<'_, AppState>,
    prefs: PreferencesPatch,
) -> Result<Settings, String> {
    let mut settings = state.settings.lock();
    if let Some(value) = prefs.clipboard_auto_clear {
        settings.clipboard_auto_clear = value;
    }
    if let Some(value) = prefs.snapshot_format {
        settings.snapshot_format = value;
    }
    if let Some(value) = prefs.save_dir {
        settings.save_dir = value;
    }
    if let Some(value) = prefs.recording_dir {
        settings.recording_dir = value;
    }
    if let Some(value) = prefs.shutter_sound {
        settings.shutter_sound = value;
    }
    if let Some(value) = prefs.custom_sound_path {
        settings.custom_sound_path = value.as_str().map(str::to_string);
    }
    if let Some(value) = prefs.flash_on_capture {
        settings.flash_on_capture = value;
    }
    if let Some(value) = prefs.hide_after_copy {
        settings.hide_after_copy = value;
    }
    if let Some(value) = prefs.auto_save_local {
        settings.auto_save_local = value;
    }
    if let Some(value) = prefs.launch_on_boot {
        settings.launch_on_boot = value;
        // 只在显式改动时写系统启动项，默认不装；取消即删除。
        os::apply_autostart(value)?;
    }
    if let Some(value) = prefs.include_cursor {
        settings.include_cursor = value;
    }
    if let Some(value) = prefs.record_system_audio {
        settings.record_system_audio = value;
    }
    if let Some(value) = prefs.record_microphone {
        settings.record_microphone = value;
    }
    if let Some(value) = prefs.after_capture {
        settings.after_capture = value;
    }
    if let Some(value) = prefs.pet_scale {
        settings.pet_scale = clamp_pet_scale(value);
    }
    persist_settings(&state.settings_path, &settings)?;
    let result = settings.clone();
    emit_settings(&app, &result);
    Ok(result)
}

fn make_models_endpoint(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err("Base URL 必须以 http:// 或 https:// 开头".into());
    }
    if trimmed.ends_with("/models") {
        return Ok(trimmed.into());
    }
    if let Some(prefix) = trimmed.strip_suffix("/chat/completions") {
        return Ok(format!("{prefix}/models"));
    }
    let version_suffix = Regex::new(r"/v\d+$").unwrap().is_match(trimmed);
    Ok(if version_suffix {
        format!("{trimmed}/models")
    } else {
        format!("{trimmed}/v1/models")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_without_pet_scale_use_the_default_size() {
        let json = r#"{"baseUrl":"","model":"","templates":[],"activeTemplateId":"builtin-default","shortcuts":[]}"#;
        let settings: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(settings.pet_scale, 100);
    }

    #[test]
    fn pet_scale_is_kept_within_its_range() {
        assert_eq!(clamp_pet_scale(10), 30);
        assert_eq!(clamp_pet_scale(150), 150);
        assert_eq!(clamp_pet_scale(300), 200);
    }
}
