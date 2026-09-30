import { describe, expect, it } from "vitest";
import {
  clampSpringVelocity,
  gestureVelocity,
  rubberBand,
  springSettled,
  stepSpring,
  targetPage,
  trackX,
  emptyWheelDetents,
  noteWheelDetent,
  wheelDetentTravel,
  wheelEase,
  WHEEL_CLUSTER_MS,
  WHEEL_EASE_MS,
  type Spring,
} from "./launchpadPaging";

describe("启动台跟手翻页", () => {
  it("范围内 1:1，拉过两端会橡皮筋，最多接近页宽的 0.42", () => {
    expect(trackX(0, 3, 0, 1000)).toBe(0);
    expect(trackX(1, 3, -40, 1000)).toBe(-1040);
    expect(trackX(0, 3, 200, 1000)).toBeCloseTo(rubberBand(200, 1000));
    expect(trackX(0, 3, 200, 1000)).toBeLessThan(200);
    const end = trackX(2, 3, -200, 1000);
    expect(end).toBeCloseTo(-2000 - rubberBand(200, 1000));
    expect(end).toBeGreaterThan(-2420);
    expect(rubberBand(1_000_000, 1000)).toBeCloseTo(420, 0);
  });

  it("越过 22% 或甩得够快就换页，方向相反时听速度，同方向不额外加页", () => {
    expect(targetPage(0, 5, -220, 0, 1000)).toBe(1);
    expect(targetPage(0, 5, -219, 0, 1000)).toBe(0);
    expect(targetPage(0, 5, -100, -0.2, 1000)).toBe(0);
    expect(targetPage(0, 5, -200, -0.4, 1000)).toBe(1);
    expect(targetPage(0, 3, -100, -6.25, 1000)).toBe(1);
    expect(targetPage(1, 5, -300, 0.8, 1000)).toBe(0);
    expect(targetPage(0, 5, -1500, -1, 1000)).toBe(2);
    expect(targetPage(0, 5, -2500, 0, 1000)).toBe(3);
    expect(targetPage(0, 3, 500, 2, 1000)).toBe(0);
    expect(targetPage(2, 3, -500, -2, 1000)).toBe(2);
    expect(clampSpringVelocity(-6.25)).toBe(-1.6);
  });

  it("同一串上报只算一格，一格翻一页，隔开再滚是下一格", () => {
    const width = 1000;
    let notch = emptyWheelDetents();
    notch = noteWheelDetent(notch, 5_000, 0);
    notch = noteWheelDetent(notch, 120, 10);
    notch = noteWheelDetent(notch, 120, 10 + WHEEL_CLUSTER_MS);
    // 一串连续上报只算一格，且一格就走满一整页
    expect(notch.count).toBe(1);
    const one = -wheelDetentTravel(notch.count, width);
    expect(one).toBe(-width);
    expect(targetPage(0, 5, one, 0, width)).toBe(1);

    notch = noteWheelDetent(notch, 80, 10 + WHEEL_CLUSTER_MS * 2 + 1);
    expect(notch.count).toBe(2);
    expect(-wheelDetentTravel(notch.count, width)).toBe(-width * 2);
    expect(targetPage(0, 5, -wheelDetentTravel(3, width), 0, width)).toBe(3);
    expect(targetPage(0, 5, -wheelDetentTravel(4, width), 0, width)).toBe(4);

    // 触控板：每 20ms 一个小 delta，一直落在 80ms 窗口里，算作一格
    let swipe = emptyWheelDetents();
    for (let t = 0; t <= 300; t += 20) {
      swipe = noteWheelDetent(swipe, 40, t);
    }
    expect(swipe.count).toBe(1);

    // 反方向是新的一个计数起点
    const reversed = noteWheelDetent(notch, -100, WHEEL_CLUSTER_MS + 10);
    expect(reversed.count).toBe(1);
    expect(gestureVelocity([{ t: 0, x: 0 }, { t: 40, x: -80 }], 200)).toBe(0);
  });

  it("滚轮滑向目标页时不冲过终点，也不往回弹", () => {
    let previous = 0;
    for (let elapsed = 0; elapsed <= WHEEL_EASE_MS; elapsed += 16) {
      const x = wheelEase(0, -1000, elapsed);
      expect(x).toBeLessThanOrEqual(previous);
      expect(x).toBeGreaterThanOrEqual(-1000);
      previous = x;
    }
    expect(wheelEase(0, -1000, WHEEL_EASE_MS)).toBe(-1000);
    expect(wheelEase(0, -1000, WHEEL_EASE_MS + 80)).toBe(-1000);
    expect(wheelEase(-1000, 0, WHEEL_EASE_MS / 2)).toBeGreaterThan(-1000);
    expect(wheelEase(-1000, 0, WHEEL_EASE_MS / 2)).toBeLessThan(0);
  });

  it("弹簧有大约 3% 的过冲，然后停稳，初速度再大也不飞出去", () => {
    const run = (start: Spring, frame: number) => {
      let spring = start;
      let peak = start.x;
      for (let step = 0; step < 5000; step += 1) {
        spring = stepSpring(spring, frame);
        peak = Math.max(peak, spring.x);
        expect(Number.isFinite(spring.x)).toBe(true);
        if (springSettled(spring)) break;
      }
      return { spring, peak };
    };
    const rested = run({ x: 0, v: 0, target: 1000 }, 16);
    expect(springSettled(rested.spring)).toBe(true);
    expect((rested.peak - 1000) / 1000).toBeGreaterThan(0.025);
    expect((rested.peak - 1000) / 1000).toBeLessThan(0.045);
    const coarse = run({ x: 0, v: 0, target: 1000 }, 32);
    expect(springSettled(coarse.spring)).toBe(true);
    expect(coarse.peak).toBeGreaterThan(1000);
    const flung = run({ x: 0, v: 1.6, target: 1000 }, 16);
    expect(springSettled(flung.spring)).toBe(true);
    expect(flung.peak).toBeLessThan(1200);
  });
});
