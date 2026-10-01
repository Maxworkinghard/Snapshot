import { describe, expect, it } from "vitest";
import { petWindowSize } from "./petSize";

describe("桌宠大小", () => {
  it("100% 是以前 30% 的视觉大小，换屏幕跟着变", () => {
    expect(petWindowSize(852, 100)).toBe(51);
    expect(petWindowSize(1080, 100)).toBe(65);
    expect(petWindowSize(600, 100)).toBe(36);
  });

  it("按百分比在默认大小上缩放", () => {
    expect(petWindowSize(1000, 100)).toBe(60);
    expect(petWindowSize(1000, 150)).toBe(90);
    expect(petWindowSize(1000, 200)).toBe(120);
  });
});
