import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { LaunchpadView } from "./launchpad";
import type {
  ActivityEntry,
  CapturableWindow,
  ClipboardState,
  PetImportResult,
  PlatformCapabilities,
  PreviousApp,
  RecordingStatus,
  Settings,
  ShortcutBinding,
  SnapshotRecord,
} from "../types";

// 前端调用后端的全部入口。浏览器预览时的假数据不在这里，见 preview.ts；
// 图片与音频不走这里，而是由页面按地址直接取，见 media.ts。

export const loadSettings = () => invoke<Settings>("load_settings");

export const savePromptSettings = (input: {
  baseUrl: string;
  model: string;
  apiKey: string | null;
  activeTemplateId: string;
  templates: Settings["templates"];
}) => invoke<Settings>("save_prompt_settings", { input });

/** 内置规则的原文（「恢复原文」用） */
export const defaultPrompt = () => invoke<string>("default_prompt");

export const fetchModels = (baseUrl: string, apiKey: string | null) =>
  invoke<string[]>("fetch_models", { baseUrl, apiKey });

export const selectPetAppearance = (id: string) => invoke<Settings>("select_pet_appearance", { id });

/** 一次导入多个；坏的那几个单独列在 failed 里，不连累其它 */
export const addPetAssets = (paths: string[]) => invoke<PetImportResult>("add_pet_assets", { paths });

export const renamePetAsset = (id: string, name: string) =>
  invoke<Settings>("rename_pet_asset", { id, name });

export const deletePetAsset = (id: string) => invoke<Settings>("delete_pet_asset", { id });

/** 后端校验、注册并落盘；有键注册不上时整组不生效、不保存，错误信息里列出这些键 */
export const saveShortcuts = (shortcuts: ShortcutBinding[]) =>
  invoke<Settings>("save_shortcuts", { shortcuts });

/** 启动时没能注册上的键（启动那一刻网页还没加载，只能事后来取） */
export const getShortcutConflicts = () => invoke<string[]>("get_shortcut_conflicts");

export const savePreferences = (prefs: Partial<Settings>) =>
  invoke<Settings>("save_preferences", { prefs });

export const getPreviousApp = () => invoke<PreviousApp>("get_previous_app");

export const listWindows = () => invoke<CapturableWindow[]>("list_capturable_windows");


/**
 * 输入框里执行会截屏的动作：后端先把输入框藏起来再动手。
 * action 为 "capture" 时截 targetId 指定的窗口；其余同全局快捷键的动作名。
 */
export const runAction = (action: string, targetId?: number) =>
  invoke<string>("run_action", { action, targetId: targetId ?? null });

/** `targetId` 指定录哪个窗口；不给就沿用「上一个应用」。停止录制时不需要它。 */
export const toggleRecording = (targetId?: number) =>
  invoke<RecordingStatus>("toggle_recording", { targetId: targetId ?? null });

export const getRecordingStatus = () => invoke<RecordingStatus>("get_recording_status");


/** 不给 templateId 就用当前生效的规则 */
export const polishText = (text: string, templateId?: string | null) =>
  invoke<string>("polish_text", { text, templateId: templateId ?? null });

export const showQuickMenu = (x: number, y: number) => invoke<void>("show_quick_menu", { x, y });

export const showLaunchpad = (x: number, y: number) => invoke<void>("show_launchpad", { x, y });

export const hideLaunchpad = () => invoke<void>("hide_launchpad");

export const launchpadState = () => invoke<LaunchpadView>("launchpad_state");

export const saveLaunchpad = (layout: { order: string[]; folders: { id: string; name: string; appIds: string[] }[] }) =>
  invoke<void>("save_launchpad", layout);

export const launchApp = (id: string) => invoke<void>("launch_app", { id });

export const onLaunchpadOpened = (callback: () => void) => listen("launchpad-opened", () => callback());

export const hideQuickMenu = () => invoke<void>("hide_quick_menu");

/** 输入框窗口按页面内容的实际高度收放 */
export const resizeQuickMenu = (width: number, height: number) =>
  invoke<void>("resize_quick_menu", { width, height });

export const showMainWindow = () => invoke<void>("show_main_window");

export const onSettingsChanged = (callback: (settings: Settings) => void) =>
  listen<Settings>("settings-changed", ({ payload }) => callback(payload));

export const onPreviousAppChanged = (callback: (app: PreviousApp) => void) =>
  listen<PreviousApp>("previous-app-changed", ({ payload }) => callback(payload));

export type CaptureFeedback = {
  flash: boolean;
  shutterSound: Settings["shutterSound"];
  customSoundPath: string | null;
};

export const onCaptureFeedback = (callback: (payload: CaptureFeedback) => void) =>
  listen<CaptureFeedback>("capture-feedback", ({ payload }) => callback(payload));

export const listSnapshots = () => invoke<SnapshotRecord[]>("list_snapshots");

export const openSnapshotsDir = () => invoke<string>("open_snapshots_dir");

export const openRecordingsDir = () => invoke<string>("open_recordings_dir");

/** 删掉选中的若干张；「全选再删」就是清空。返回删完后的列表 */
export const deleteSnapshots = (ids: string[]) => invoke<SnapshotRecord[]>("delete_snapshots", { ids });

/** 把历史里的一张重新放回剪贴板 */
export const copySnapshot = (id: string) => invoke<string>("copy_snapshot", { id });

/** 截图入库或被删后，后端广播一次，列表据此刷新 */
export const onSnapshotsChanged = (callback: () => void) => listen("snapshots-changed", () => callback());

export const listActivity = () => invoke<ActivityEntry[]>("list_activity");

export const onActivity = (callback: (entry: ActivityEntry) => void) =>
  listen<ActivityEntry>("activity-added", ({ payload }) => callback(payload));

export const getClipboardState = () => invoke<ClipboardState | null>("get_clipboard_state");

export const onClipboardChanged = (callback: (state: ClipboardState | null) => void) =>
  listen<ClipboardState | null>("clipboard-changed", ({ payload }) => callback(payload));

export const clearClipboardNow = () => invoke<void>("clear_clipboard_now");

/** 图留在剪贴板里，只是不再倒计时 */
export const keepClipboard = () => invoke<void>("keep_clipboard");

export const copyText = (text: string) => invoke<void>("copy_text", { text });

export const readClipboardText = () => invoke<string>("read_clipboard_text");

export const onRecordingChanged = (callback: (status: RecordingStatus) => void) =>
  listen<RecordingStatus>("recording-changed", ({ payload }) => callback(payload));

export const onPaletteOpened = (callback: () => void) => listen("palette-opened", () => callback());

export const platformCapabilities = () => invoke<PlatformCapabilities>("platform_capabilities");
