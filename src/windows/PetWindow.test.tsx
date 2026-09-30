import { act, fireEvent, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { emit } from "@tauri-apps/api/event";
import type { PetAsset, Settings } from "../types";
import { setupTauriMock } from "../test/tauri";

const grok: PetAsset = {
  id: "pet-grok",
  name: "grok_pixel",
  path: "",
  entry: "gifs/grok_idle.gif",
  animations: [
    "gifs/grok_idle.gif",
    "gifs/grok_jump.gif",
    "gifs/grok_walk_left.gif",
    "gifs/grok_walk_right.gif",
    "gifs/grok_wave.gif",
  ],
};

function settingsWith(asset: PetAsset) {
  return {
    baseUrl: "",
    model: "",
    hasApiKey: false,
    templates: [{ id: "builtin-default", name: "默认", content: "x", builtin: true }],
    activeTemplateId: "builtin-default",
    selectedAppearanceId: asset.id,
    petAssets: [asset],
    petScale: 100,
    shortcuts: [],
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
  } as Settings;
}

async function renderPet(asset: PetAsset = grok) {
  vi.useFakeTimers();
  const calls: string[] = [];
  setupTauriMock(
    (command) => {
      calls.push(command);
      if (command === "load_settings") return settingsWith(asset);
      if (command === "get_previous_app") return { id: null, pid: null, name: "", title: "" };
      return undefined;
    },
    { currentWindow: "pet", shouldMockEvents: true },
  );
  // petSync 在模块加载时判断是不是 Tauri 环境，要在装好 mock 之后再加载
  vi.resetModules();
  const { PetWindow, CLICK_DELAY_MS } = await import("./PetWindow");
  const view = render(<PetWindow />);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
  return { ...view, calls, delay: CLICK_DELAY_MS };
}

/** 桌宠上看得见的那张图：文件名（解码后）和是否镜像 */
function shown(container: HTMLElement) {
  const visible = [...container.querySelectorAll<HTMLImageElement>(".pet-orb img")].filter((img) => !img.hidden);
  expect(visible).toHaveLength(1);
  const [img] = visible;
  return { file: decodeURIComponent(img.src).split("/").pop(), flipped: img.classList.contains("pet-flip"), img };
}

/** 系统拖着窗口走：依次移到这些物理坐标 */
async function moveTo(...points: Array<[number, number]>) {
  for (const [x, y] of points) {
    await act(async () => {
      await emit("tauri://move", { x, y });
    });
  }
}

async function elapse(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

function click(container: HTMLElement) {
  fireEvent.pointerDown(container.querySelector(".pet-window")!, { button: 0, clientX: 10, clientY: 10 });
  fireEvent.pointerUp(window);
}

describe("桌宠：点击切换，平时停在待机", () => {
  it("不会自己换动作", async () => {
    const { container } = await renderPet();
    await elapse(8000);
    expect(shown(container).file).toBe("grok_idle.gif");
    await elapse(8000);
    expect(shown(container).file).toBe("grok_idle.gif");
  });

  it("点一下换到下一个非走路动作，3 秒后回到待机，再点换到再下一个", async () => {
    const { container, calls, delay } = await renderPet();
    expect(shown(container).file).toBe("grok_idle.gif");

    await act(async () => click(container));
    expect(shown(container).file).toBe("grok_idle.gif");
    await elapse(delay);
    expect(shown(container).file).toBe("grok_jump.gif");
    expect(calls).not.toContain("show_launchpad");
    await elapse(2999);
    expect(shown(container).file).toBe("grok_jump.gif");
    await elapse(1);
    expect(shown(container).file).toBe("grok_idle.gif");

    await act(async () => click(container));
    await elapse(delay);
    expect(shown(container).file).toBe("grok_wave.gif");
    await elapse(3000);
    expect(shown(container).file).toBe("grok_idle.gif");

    await act(async () => click(container));
    await elapse(delay);
    expect(shown(container).file).toBe("grok_jump.gif");
  });

  it("3 秒内再点一下会换到下一个，并重新计算回待机的时间", async () => {
    const { container, delay } = await renderPet();
    await act(async () => click(container));
    await elapse(delay);
    expect(shown(container).file).toBe("grok_jump.gif");
    await elapse(2000);
    await act(async () => click(container));
    await elapse(delay);
    expect(shown(container).file).toBe("grok_wave.gif");
    await elapse(2000);
    expect(shown(container).file).toBe("grok_wave.gif");
    await elapse(1000);
    expect(shown(container).file).toBe("grok_idle.gif");
  });

  it("双击打开启动台，动作留在待机，下一次单击仍从下一个动作开始", async () => {
    const { container, calls, delay } = await renderPet();
    await act(async () => click(container));
    await elapse(120);
    await act(async () => click(container));
    await elapse(delay);
    expect(shown(container).file).toBe("grok_idle.gif");
    expect(calls).toContain("show_launchpad");

    calls.length = 0;
    await act(async () => click(container));
    await elapse(delay);
    expect(calls).not.toContain("show_launchpad");
    expect(shown(container).file).toBe("grok_jump.gif");
  });

  it("除了待机没有别的可切换动作时，点击仍停在待机", async () => {
    const { container, delay } = await renderPet({
      ...grok,
      animations: ["gifs/grok_idle.gif", "gifs/grok_walk_left.gif", "gifs/grok_walk_right.gif"],
    });
    await act(async () => click(container));
    await elapse(delay + 3000);
    expect(shown(container).file).toBe("grok_idle.gif");
  });
});

describe("桌宠：拖动时走路", () => {
  it("往右拖显示向右走，往左拖显示向左走，停 0.25 秒回到原来的动作", async () => {
    const { container } = await renderPet();
    await moveTo([100, 100], [106, 101], [112, 100]);
    expect(shown(container)).toMatchObject({ file: "grok_walk_right.gif", flipped: false });

    await moveTo([104, 100], [96, 101]);
    expect(shown(container)).toMatchObject({ file: "grok_walk_left.gif", flipped: false });

    await elapse(200);
    expect(shown(container).file).toBe("grok_walk_left.gif");
    await elapse(100);
    expect(shown(container).file).toBe("grok_idle.gif");
  });

  it("一直在拖就一直走，不会中途变回去", async () => {
    const { container } = await renderPet();
    await moveTo([0, 0]);
    for (let x = 5; x <= 50; x += 5) {
      await elapse(200);
      await moveTo([x, 0]);
    }
    expect(shown(container).file).toBe("grok_walk_right.gif");
  });

  it("走路图一开始就藏在同一格里，拖起来直接换上，不重新加载", async () => {
    const { container } = await renderPet();
    const images = [...container.querySelectorAll<HTMLImageElement>(".pet-orb img")];
    const walkRight = images.find((img) => decodeURIComponent(img.src).endsWith("grok_walk_right.gif"));
    expect(walkRight?.hidden).toBe(true);
    expect(decodeURIComponent(walkRight!.src)).toContain("pet-walk/");

    await moveTo([0, 0], [8, 0]);
    expect(shown(container).img).toBe(walkRight);
    expect(container.querySelectorAll(".pet-orb img")).toHaveLength(images.length);
  });

  it("上下拖不算走路", async () => {
    const { container } = await renderPet();
    await moveTo([0, 0], [1, 12], [2, 30]);
    expect(shown(container).file).toBe("grok_idle.gif");
  });

  it("只有向右走的图时，往左拖把它镜像过来", async () => {
    const { container } = await renderPet({
      ...grok,
      animations: ["gifs/grok_idle.gif", "gifs/grok_walk_right.gif"],
    });
    await moveTo([50, 0], [40, 0]);
    expect(shown(container)).toMatchObject({ file: "grok_walk_right.gif", flipped: true });
  });

  it("包里没有走路动作时拖动照常显示原来的动作", async () => {
    const { container } = await renderPet({
      ...grok,
      animations: ["gifs/grok_idle.gif", "gifs/grok_wave.gif"],
    });
    await moveTo([0, 0], [20, 0]);
    expect(shown(container).file).toBe("grok_idle.gif");
  });
});
