import { act, fireEvent, render, screen } from "@testing-library/react";
import { emit } from "@tauri-apps/api/event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { rubberBand, WHEEL_CLUSTER_MS, WHEEL_EASE_MS, WHEEL_QUIET_MS } from "../lib/launchpadPaging";
import { setupTauriMock, invokeArgs } from "../test/tauri";
import { LaunchpadWindow } from "./LaunchpadWindow";

const state = {
  apps: [
    { id: "a", name: "记事本" },
    { id: "b", name: "画图" },
    { id: "c", name: "计算器" },
  ],
  items: [
    {
      kind: "folder" as const,
      id: "f1",
      name: "办公",
      apps: [
        { id: "a", name: "记事本" },
        { id: "b", name: "画图" },
      ],
    },
    { kind: "app" as const, id: "c", name: "计算器" },
  ],
};

async function openPad(calls: Array<{ command: string; id?: string }>) {
  setupTauriMock(
    (command, payload) => {
      const args = invokeArgs(payload);
      calls.push({ command, id: typeof args.id === "string" ? args.id : undefined });
      if (command === "launchpad_state") return state;
      return undefined;
    },
    { currentWindow: "launchpad", shouldMockEvents: true },
  );
  render(<LaunchpadWindow />);
  await act(async () => {
    await emit("launchpad-opened");
  });
}

const nativeAnimate = Element.prototype.animate;

/** 记下 root.animate 收到的关键帧和选项，用来断言启动台整块的进出场 */
function spyAnimate() {
  const calls: Keyframe[][] = [];
  const options: KeyframeAnimationOptions[] = [];
  const fake = { finished: Promise.resolve(), cancel: () => {}, play: () => {}, pause: () => {} };
  Object.defineProperty(Element.prototype, "animate", {
    configurable: true,
    writable: true,
    value: function (this: Element, frames: Keyframe[], opts?: KeyframeAnimationOptions) {
      // 只关心启动台整块的进出场，翻页/拖动那些内部动画不算
      if (this.classList?.contains("launchpad")) {
        calls.push(frames);
        options.push(opts ?? {});
      }
      return fake as unknown as Animation;
    },
  });
  return { calls, options };
}

function restoreAnimate() {
  if (nativeAnimate) Element.prototype.animate = nativeAnimate;
  else Reflect.deleteProperty(Element.prototype, "animate");
}

describe("启动台进场与退场", () => {
  it("每次打开都整块淡入，不是硬切", async () => {
    const spy = spyAnimate();
    await openPad([]);
    expect(spy.calls).toEqual([[{ opacity: 0 }, { opacity: 1 }]]);

    await act(async () => {
      await emit("launchpad-opened");
    });
    expect(spy.calls).toHaveLength(2);
    restoreAnimate();
  });

  it("减弱动效时仍然淡入，只是短一些", async () => {
    document.documentElement.dataset.motion = "reduced";
    const spy = spyAnimate();
    await openPad([]);
    expect(spy.calls).toHaveLength(1);
    expect(spy.options[0].duration).toBe(180);
    restoreAnimate();
  });

  it("关闭时先淡出再收起窗口，不是硬切", async () => {
    const calls: Array<{ command: string }> = [];
    const spy = spyAnimate();
    await openPad(calls);
    expect(calls.some((call) => call.command === "hide_launchpad")).toBe(false);

    fireEvent.keyDown(window, { key: "Escape" });
    // 先播退场动画，窗口这时还收着
    expect(spy.calls).toHaveLength(2);
    expect(spy.calls[1]).toEqual([{ opacity: 1 }, { opacity: 0 }]);
    expect(calls.some((call) => call.command === "hide_launchpad")).toBe(false);

    // 动画播完才真正收起
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(calls.some((call) => call.command === "hide_launchpad")).toBe(true);
    restoreAnimate();
  });

  it("点应用启动时也淡出，不会闪一下才消失", async () => {
    const spy = spyAnimate();
    await openPad([]);
    const tile = await screen.findByRole("button", { name: "计算器" });
    fireEvent.pointerDown(tile, { button: 0, clientX: 4, clientY: 4 });
    await act(async () => {
      fireEvent.pointerUp(window);
    });
    expect(spy.calls).toHaveLength(2);
    expect(spy.calls[1]).toEqual([{ opacity: 1 }, { opacity: 0 }]);
    restoreAnimate();
  });

  it("减弱动效时退场也保留，只是短一些", async () => {
    document.documentElement.dataset.motion = "reduced";
    const calls: Array<{ command: string }> = [];
    const spy = spyAnimate();
    await openPad(calls);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(spy.calls[1]).toEqual([{ opacity: 1 }, { opacity: 0 }]);
    expect(spy.options[1].duration).toBe(180);
    restoreAnimate();
  });
});

describe("启动台窗口", () => {
  it("文件夹里的应用不在外面，搜索能找到，点一下会启动", async () => {
    const calls: Array<{ command: string; id?: string }> = [];
    await openPad(calls);
    expect(await screen.findByRole("button", { name: "办公" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "计算器" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "记事本" })).toBeNull();

    fireEvent.change(screen.getByRole("textbox", { name: "搜索应用" }), { target: { value: "记事" } });
    const note = await screen.findByRole("button", { name: "记事本" });
    expect(screen.queryByRole("button", { name: "计算器" })).toBeNull();
    fireEvent.pointerDown(note, { button: 0, clientX: 4, clientY: 4 });
    fireEvent.pointerUp(window);
    expect(calls).toContainEqual({ command: "launch_app", id: "a" });
  });

  it("点开文件夹能看到里面的应用，按 Esc 先关上文件夹", async () => {
    const calls: Array<{ command: string; id?: string }> = [];
    await openPad(calls);
    const folder = await screen.findByRole("button", { name: "办公" });
    fireEvent.pointerDown(folder, { button: 0, clientX: 4, clientY: 4 });
    fireEvent.pointerUp(window);
    expect(await screen.findByRole("button", { name: "记事本" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "画图" })).toBeTruthy();

    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("button", { name: "记事本" })).toBeNull();
    expect(calls.some((call) => call.command === "hide_launchpad")).toBe(false);

    fireEvent.keyDown(window, { key: "Escape" });
    expect(calls.some((call) => call.command === "hide_launchpad")).toBe(true);
  });
});

const paged = {
  apps: Array.from({ length: 71 }, (_, index) => ({ id: `p${index}`, name: `应用${index}` })),
  items: Array.from({ length: 71 }, (_, index) => ({ kind: "app" as const, id: `p${index}`, name: `应用${index}` })),
};

afterEach(() => {
  delete document.documentElement.dataset.motion;
  // 断言失败时用例里的 restoreAnimate() 不会执行，这里兜底，别让桩漏到下一个用例
  restoreAnimate();
  vi.restoreAllMocks();
});

function setViewport(width: number, height: number) {
  vi.spyOn(window, "innerWidth", "get").mockReturnValue(width);
  vi.spyOn(window, "innerHeight", "get").mockReturnValue(height);
}

function trackX() {
  const value = document.querySelector<HTMLElement>(".launchpad-track")?.style.transform ?? "";
  return Number(/translate3d\(([-\d.]+)px/.exec(value)?.[1] ?? NaN);
}

describe("启动台翻页", () => {
  async function openPaged(record?: Array<{ command: string; id?: string }>) {
    setViewport(1000, 1200);
    setupTauriMock(
      (command, payload) => {
        const args = invokeArgs(payload);
        record?.push({ command, id: typeof args.id === "string" ? args.id : undefined });
        if (command === "launchpad_state") return paged;
        return undefined;
      },
      { currentWindow: "launchpad", shouldMockEvents: true },
    );
    render(<LaunchpadWindow />);
    await act(async () => {
      await emit("launchpad-opened");
    });
    expect(await screen.findByRole("button", { name: "应用0" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "应用35" })).toBeNull();
    expect(document.querySelector('[aria-label="应用35"]')).toBeTruthy();
    expect(trackX()).toBe(0);
  }

  function currentPage() {
    const current = screen.getAllByRole("button").find((button) => button.classList.contains("is-current"));
    return current?.getAttribute("aria-label");
  }

  async function settleWheel() {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(WHEEL_QUIET_MS);
      await vi.advanceTimersByTimeAsync(2000);
    });
  }

  async function nextNotch() {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(WHEEL_CLUSTER_MS + 10);
    });
  }

  it("滚轮和拖动都跟手，松手后翻一页，一次手势不会连翻", async () => {
    await openPaged();
    vi.useFakeTimers();

    // 一串连续上报（120 / 400 / 5000 是同一个 notch 被拆开的几次）只算一格，
    // 一格就走一页。滚完立刻亮页码，位移随后滑过去。
    fireEvent.wheel(window, { deltaY: 120 });
    fireEvent.wheel(window, { deltaY: 400 });
    fireEvent.wheel(window, { deltaY: 5_000 });
    expect(currentPage()).toBe("第 2 页");
    expect(trackX()).toBe(0);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(120);
    });
    const mid = trackX();
    expect(mid).toBeLessThan(0);
    expect(mid).toBeGreaterThan(-1000);
    await settleWheel();
    expect(trackX()).toBeCloseTo(-1000);
    expect(currentPage()).toBe("第 2 页");
    expect(screen.queryByRole("button", { name: "应用0" })).toBeNull();
    expect(document.querySelector(".launchpad-page[aria-hidden='true']")).toBeTruthy();

    // 再滚一格：继续往下走一页
    fireEvent.wheel(window, { deltaY: 120 });
    expect(currentPage()).toBe("第 3 页");
    await settleWheel();
    expect(trackX()).toBeCloseTo(-2000);

    // 往回滚一格：回到上一页
    fireEvent.wheel(window, { deltaX: -120, deltaY: 10 });
    expect(currentPage()).toBe("第 2 页");
    await settleWheel();
    expect(trackX()).toBeCloseTo(-1000);
    expect(currentPage()).toBe("第 2 页");
    // 「应用0」在第 1 页，第 2 页上看不到它；第 2 页的第一项是「应用35」
    expect(screen.queryByRole("button", { name: "应用0" })).toBeNull();
    expect(screen.getByRole("button", { name: "应用35" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "第 3 页" }));
    expect(currentPage()).toBe("第 3 页");

    await act(async () => {
      await emit("launchpad-opened");
    });
    expect(currentPage()).toBe("第 1 页");
    expect(trackX()).toBe(0);
  });

  it("鼠标隔多久滚一格都能翻页，不必抢在 400ms 内连滚两下", async () => {
    await openPaged();
    vi.useFakeTimers();
    // 以前要两格才翻页，而停手 400ms 就重置计数，所以间隔大的正常滚法永远翻不了。
    for (const gap of [120, 300, 500, 800]) {
      await act(async () => {
        await emit("launchpad-opened");
      });
      await settleWheel();
      expect(currentPage()).toBe("第 1 页");
      fireEvent.wheel(window, { deltaY: 100 });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(gap);
      });
      await settleWheel();
      // 滚一格就是一页，不依赖下一次滚得多快
      expect(currentPage()).toBe("第 2 页");
    }
  });

  it("触控板那样连续小步滑动也能翻页，不会一直停在原地", async () => {
    await openPaged();
    vi.useFakeTimers();
    // 触控板每十几毫秒发一个小 delta，且始终落在 80ms 聚类窗口内。
    // 只按窗口聚类的话这串会被当成同一格，攒不出第二格，滚到底也不动。
    for (let step = 0; step < 24; step += 1) {
      fireEvent.wheel(window, { deltaY: 40 });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(20);
      });
    }
    // 修之前这里会一直停在「第 1 页」。一串上报封顶 160ms 算一格，
    // 480ms 的连续滑动刚好吃满两格 → 翻一页。
    expect(currentPage()).not.toBe("第 1 页");
    await settleWheel();
    expect(currentPage()).toBe("第 2 页");
    expect(trackX()).toBeCloseTo(-1000);
  });

  it("滚轮滑到下一页时不往回弹，途中再滚一格也不会停在半页", async () => {
    await openPaged();
    vi.useFakeTimers();
    fireEvent.wheel(window, { deltaY: 120 });

    // 一格一页：单调靠近 -1000，不冲过终点
    let previous = 0;
    for (let step = 0; step < 8; step += 1) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(16);
      });
      const x = trackX();
      expect(x).toBeLessThanOrEqual(previous + 0.01);
      expect(x).toBeGreaterThanOrEqual(-1000);
      previous = x;
    }
    expect(previous).toBeLessThan(0);
    expect(previous).toBeGreaterThan(-1000);

    // 滑到一半再滚一格：目标改成两页，继续往前，不回弹也不停在半页
    await nextNotch();
    fireEvent.wheel(window, { deltaY: 120 });
    // 第二格 → 目标是两页之后。此刻还在第一段的路上，方向继续往前，不往回弹
    expect(currentPage()).toBe("第 3 页");
    const interrupted = trackX();
    expect(interrupted).toBeLessThanOrEqual(previous + 0.01);
    expect(interrupted).toBeGreaterThan(-2000);

    await settleWheel();
    expect(trackX()).toBeCloseTo(-2000);
    expect(currentPage()).toBe("第 3 页");
  });

  it("按住 Ctrl 的滚轮、搜索和打开的文件夹都不翻页", async () => {
    const calls: Array<{ command: string }> = [];
    setupTauriMock(
      (command) => {
        calls.push({ command });
        if (command === "launchpad_state") {
          return {
            apps: paged.apps,
            items: [
              {
                kind: "folder" as const,
                id: "f1",
                name: "办公",
                apps: [
                  { id: "a", name: "记事本" },
                  { id: "b", name: "画图" },
                ],
              },
              ...paged.items,
            ],
          };
        }
        return undefined;
      },
      { currentWindow: "launchpad", shouldMockEvents: true },
    );
    render(<LaunchpadWindow />);
    await act(async () => {
      await emit("launchpad-opened");
    });
    expect(await screen.findByRole("button", { name: "办公" })).toBeTruthy();

    fireEvent.wheel(window, { deltaY: 120, ctrlKey: true });
    expect(currentPage()).toBe("第 1 页");

    fireEvent.change(screen.getByRole("textbox", { name: "搜索应用" }), { target: { value: "记事" } });
    fireEvent.wheel(window, { deltaY: 120 });
    fireEvent.change(screen.getByRole("textbox", { name: "搜索应用" }), { target: { value: "" } });
    expect(currentPage()).toBe("第 1 页");

    fireEvent.pointerDown(screen.getByRole("button", { name: "办公" }), { button: 0, clientX: 4, clientY: 4 });
    fireEvent.pointerUp(window);
    expect(await screen.findByRole("button", { name: "记事本" })).toBeTruthy();
    fireEvent.wheel(window, { deltaY: 120 });
    fireEvent.keyDown(window, { key: "Escape" });
    expect(currentPage()).toBe("第 1 页");
    expect(calls.some((call) => call.command === "hide_launchpad")).toBe(false);
  });

  it("横向拖动跟手，横拖图标不打开，点空白才关闭", async () => {
    const calls: Array<{ command: string; id?: string }> = [];
    await openPaged(calls);
    vi.useFakeTimers();
    vi.spyOn(performance, "now").mockReturnValue(5_000);
    document.documentElement.dataset.motion = "reduced";
    const app = screen.getByRole("button", { name: "应用0" });
    fireEvent.pointerDown(app, { button: 0, clientX: 300, clientY: 200 });
    fireEvent.pointerMove(window, { clientX: 140, clientY: 202 });
    fireEvent.pointerUp(window, { button: 0, clientX: 140, clientY: 202 });
    expect(calls.some((call) => call.command === "launch_app")).toBe(false);
    expect(document.querySelector(".launchpad-ghost")).toBeNull();
    expect(currentPage()).toBe("第 1 页");

    fireEvent.pointerDown(app, { button: 0, clientX: 300, clientY: 200 });
    fireEvent.pointerMove(window, { clientX: 304, clientY: 280 });
    expect(document.querySelector(".launchpad-ghost")).toBeTruthy();
    fireEvent.pointerUp(window, { button: 0, clientX: 304, clientY: 280 });
    expect(calls.some((call) => call.command === "launch_app")).toBe(false);

    const viewport = document.querySelector(".launchpad-viewport");
    if (!viewport) throw new Error("缺少翻页视口");
    fireEvent.pointerDown(viewport, { button: 0, clientX: 200, clientY: 360 });
    fireEvent.pointerMove(window, { clientX: 380, clientY: 360 });
    expect(trackX()).toBeCloseTo(rubberBand(180, 1000));
    fireEvent.pointerUp(window, { button: 0, clientX: 380, clientY: 360 });
    expect(currentPage()).toBe("第 1 页");
    expect(trackX()).toBe(0);

    fireEvent.pointerDown(viewport, { button: 0, clientX: 500, clientY: 360 });
    fireEvent.pointerMove(window, { clientX: 100, clientY: 360 });
    expect(trackX()).toBe(-400);
    fireEvent.pointerUp(window, { button: 0, clientX: 100, clientY: 360 });
    expect(currentPage()).toBe("第 2 页");
    expect(trackX()).toBe(-1000);

    const root = document.querySelector(".launchpad");
    if (!root) throw new Error("缺少启动台");
    fireEvent.pointerDown(root, { button: 0, clientX: 8, clientY: 8 });
    fireEvent.pointerUp(window, { button: 0, clientX: 10, clientY: 10 });
    expect(calls.some((call) => call.command === "hide_launchpad")).toBe(true);
  });

  it("鼠标侧键前进后退会翻页，并且不会被当成普通点击", async () => {
    await openPaged();
    const down = new MouseEvent("pointerdown", { button: 4, bubbles: true, cancelable: true });
    window.dispatchEvent(down);
    expect(down.defaultPrevented).toBe(true);
    expect(currentPage()).toBe("第 1 页");

    fireEvent.pointerUp(window, { button: 4 });
    expect(currentPage()).toBe("第 2 页");
    fireEvent.pointerUp(window, { button: 3 });
    expect(currentPage()).toBe("第 1 页");
    expect(screen.getByRole("button", { name: "应用0" })).toBeTruthy();
  });
});
