import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { InvokeArgs } from "@tauri-apps/api/core";
import type { PlatformCapabilities, Settings } from "../types";
import { setupTauriMock, type IPCHandler } from "../test/tauri";

/**
 * B2 拆页后的回归网：逐个导航到从 App.tsx 搬出的页面，确认它们在新位置
 * 仍然真的渲染出来（编译通过不等于渲染通过）。找出每个页面独有的文案做断言。
 */

const settings: Settings = {
  baseUrl: "",
  model: "",
  hasApiKey: false,
  templates: [],
  activeTemplateId: "builtin-default",
  selectedAppearanceId: "app-icon",
  petAssets: [],
  petScale: 100,
  shortcuts: [
    { action: "snapshot", accelerator: "Alt+Shift+2" },
    { action: "record", accelerator: null },
  ],
  clipboardAutoClear: "60s",
  snapshotFormat: "png",
  saveDir: "",
  recordingDir: "",
  shutterSound: "crisp",
  customSoundPath: null,
  flashOnCapture: true,
  hideAfterCopy: false,
  autoSaveLocal: true,
  launchOnBoot: false,
  includeCursor: false,
  recordSystemAudio: false,
  recordMicrophone: false,
  afterCapture: "clipboard",
};

const capabilities: PlatformCapabilities = {
  os: "windows",
  displayServer: "n/a",
  recording: { available: true },
  recordingSystemAudio: { available: true },
  recordingMicrophone: { available: true },
  autostart: { available: true },
  includeCursor: { available: true },
};

function handler(command: string) {
  switch (command) {
    case "load_settings":
      return settings;
    case "platform_capabilities":
      return capabilities;
    case "list_snapshots":
      return [];
    case "get_recording_status":
      return { active: false, target: null, startedAt: null };
    default:
      return undefined;
  }
}

async function renderApp(handle: IPCHandler = handler) {
  setupTauriMock(handle, { currentWindow: "main", shouldMockEvents: true });
  vi.resetModules();
  const { App } = await import("../App");
  render(<App />);
  // 等设置加载完（加载态消失，出现侧边栏导航）
  await screen.findByRole("button", { name: "快捷操作" });
}

describe("B2 拆页后逐页渲染", () => {
  it("默认落在快捷操作页，能看到快捷键行", async () => {
    await renderApp();
    expect(
      await screen.findByRole("button", { name: "快捷键：窗口快照，当前按键：Alt+Shift+2" }),
    ).toBeTruthy();
  });

  it.each([
    [/^Prompt$/, "规则"],
    [/^快照历史/, "还没有快照"],
    [/^桌面伴侣/, "全部形象"],
    [/^偏好设置/, "截屏与行为"],
    [/^主题库/, "布局"],
  ])("导航到「%s」能渲染出该页独有内容", async (navLabel, marker) => {
    await renderApp();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: navLabel }));
    // 换页不等旧页淡出，新页内容应当马上出现
    expect(await screen.findAllByText(marker, { exact: false })).not.toHaveLength(0);
  });

  it("偏好设置里拖桌宠大小：存下百分比，旁边跟着显示", async () => {
    const calls: Array<{ command: string; payload?: InvokeArgs }> = [];
    await renderApp((command, payload) => {
      calls.push({ command, payload });
      if (command === "save_preferences") return { ...settings, ...(payload as { prefs: Partial<Settings> }).prefs };
      return handler(command);
    });
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /^偏好设置/ }));
    const slider = await screen.findByRole("slider", { name: "大小" });
    fireEvent.change(slider, { target: { value: "150" } });
    expect(await screen.findByText("150%")).toBeTruthy();
    await waitFor(() =>
      expect(calls.find((call) => call.command === "save_preferences")?.payload).toEqual({ prefs: { petScale: 150 } }),
    );
    expect(screen.getByRole("button", { name: "恢复默认" })).toBeTruthy();
  });

  it("主题库里换成「时间线」：侧栏消失，标题栏出现「今天」", async () => {
    await renderApp();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /^主题库/ }));
    await user.click(await screen.findByRole("radio", { name: /时间线/ }));
    expect(await screen.findByRole("button", { name: "今天" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "快捷操作" })).toBeNull();
    await waitFor(() => expect(window.localStorage.getItem("snapshot-layout")).toBe("timeline"));
    window.localStorage.removeItem("snapshot-layout");
  });

  it("「时间线」的设置页里也能调桌宠大小", async () => {
    await renderApp();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /^主题库/ }));
    await user.click(await screen.findByRole("radio", { name: /时间线/ }));
    await user.click(await screen.findByRole("button", { name: "设置" }));
    expect(await screen.findByRole("slider", { name: "大小" })).toBeTruthy();
    window.localStorage.removeItem("snapshot-layout");
  });
});
// jsdom 未实现 Element.prototype.scrollTo；App 换页后会调用它回到顶部。
if (!Element.prototype.scrollTo) {
  Element.prototype.scrollTo = () => {};
}
