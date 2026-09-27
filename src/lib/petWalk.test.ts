import { describe, expect, it } from "vitest";
import { cycleEntries, isWalkEntry, nextDragWalk, pickWalk, STANDING, walkDirection } from "./petWalk";

// 下面的动作名都取自实际的素材包
const grok = [
  "grok_pixel/gifs/grok_idle.gif",
  "grok_pixel/gifs/grok_jump.gif",
  "grok_pixel/gifs/grok_walk_left.gif",
  "grok_pixel/gifs/grok_walk_right.gif",
  "grok_pixel/gifs/grok_wave.gif",
];
const clerk = [
  "yamada_pixel/gifs/clerk_idle.gif",
  "yamada_pixel/gifs/idle.gif",
  "yamada_pixel/gifs/clerk_walk_left.gif",
  "yamada_pixel/gifs/clerk_walk_left_move.gif",
  "yamada_pixel/gifs/clerk_wave.gif",
  "yamada_pixel/gifs/smoke_sit.gif",
  "yamada_pixel/gifs/walk_left.gif",
  "yamada_pixel/gifs/walk_left_move.gif",
  "yamada_pixel/gifs/walk_right.gif",
  "yamada_pixel/gifs/walk_right_move.gif",
  "yamada_pixel/gifs/clerk_walk_right.gif",
  "yamada_pixel/gifs/clerk_walk_right_move.gif",
];
const yamadaQ = ["yamada-q-idle.gif", "yamada-q-running-right.gif", "yamada-q-running-left.gif", "yamada-q-running.gif", "yamada-q-waving.gif"];
const clawd = ["clawd_gifs/clawd-idle.gif", "clawd_gifs/clawd-crabwalk.gif", "clawd_gifs/clawd-dance.gif"];
const nashorV2 = [
  "nashor_pet_v2/gifs/nashor_idle.gif",
  "nashor_pet_v2/gifs/nashor_attack_v2.gif",
  "nashor_pet_v2/gifs/nashor_burrow_left.gif",
  "nashor_pet_v2/gifs/nashor_burrow_right.gif",
  "nashor_pet_v2/gifs/nashor_sway.gif",
];
const nashorV1 = ["nashor_pet_v1/gifs/nashor_idle.gif", "nashor_pet_v1/gifs/nashor_attack.gif", "nashor_pet_v1/gifs/nashor_sway.gif"];

describe("走路动作的识别", () => {
  it("名字带 walk / run / move / 走 / 跑、或单独写了朝向的算走路，朝向看 left / right / 左 / 右", () => {
    expect(isWalkEntry("grok_pixel/gifs/grok_walk_left.gif")).toBe(true);
    expect(isWalkEntry("yamada-q-running.gif")).toBe(true);
    expect(isWalkEntry("clawd-crabwalk.gif")).toBe(true);
    expect(isWalkEntry("向左走.gif")).toBe(true);
    expect(isWalkEntry("grok_pixel/gifs/grok_wave.gif")).toBe(false);
    // 钻地往左 / 往右是 nashor 的走法：名字里没有 walk，但写了朝向
    expect(isWalkEntry("nashor_burrow_left.gif")).toBe(true);
    expect(isWalkEntry("nashor_burrow_right.gif")).toBe(true);
    // 单词里恰好含 right 的不算朝向
    expect(isWalkEntry("cat_frightened.gif")).toBe(false);
    expect(walkDirection("grok_walk_left.gif")).toBe("left");
    expect(walkDirection("yamada-q-running-right.gif")).toBe("right");
    expect(walkDirection("向右跑.gif")).toBe("right");
    expect(walkDirection("clawd-crabwalk.gif")).toBeNull();
  });

  it("点击和自动轮换不经过走路动作；整包都是走路时照常轮换", () => {
    expect(cycleEntries(grok)).toEqual([
      "grok_pixel/gifs/grok_idle.gif",
      "grok_pixel/gifs/grok_jump.gif",
      "grok_pixel/gifs/grok_wave.gif",
    ]);
    expect(cycleEntries(yamadaQ)).toEqual(["yamada-q-idle.gif", "yamada-q-waving.gif"]);
    expect(cycleEntries(nashorV2)).toEqual([
      "nashor_pet_v2/gifs/nashor_idle.gif",
      "nashor_pet_v2/gifs/nashor_attack_v2.gif",
      "nashor_pet_v2/gifs/nashor_sway.gif",
    ]);
    expect(cycleEntries(["walk_left.gif", "walk_right.gif"])).toEqual(["walk_left.gif", "walk_right.gif"]);
  });
});

describe("拖动时播哪个走路动作", () => {
  it("有同方向的就用同方向的", () => {
    expect(pickWalk(grok, "left", grok[0])).toEqual({ entry: "grok_pixel/gifs/grok_walk_left.gif", flip: false });
    expect(pickWalk(grok, "right", grok[0])).toEqual({ entry: "grok_pixel/gifs/grok_walk_right.gif", flip: false });
    expect(pickWalk(yamadaQ, "left", yamadaQ[0])).toEqual({ entry: "yamada-q-running-left.gif", flip: false });
    expect(pickWalk(nashorV2, "left", nashorV2[0])).toEqual({ entry: "nashor_pet_v2/gifs/nashor_burrow_left.gif", flip: false });
    expect(pickWalk(nashorV2, "right", nashorV2[0])).toEqual({ entry: "nashor_pet_v2/gifs/nashor_burrow_right.gif", flip: false });
  });

  it("一个包里有两套装扮时跟着当前那套走，并且优先原地走的版本", () => {
    expect(pickWalk(clerk, "left", "yamada_pixel/gifs/clerk_idle.gif")?.entry).toBe("yamada_pixel/gifs/clerk_walk_left.gif");
    expect(pickWalk(clerk, "right", "yamada_pixel/gifs/clerk_wave.gif")?.entry).toBe("yamada_pixel/gifs/clerk_walk_right.gif");
    expect(pickWalk(clerk, "left", "yamada_pixel/gifs/idle.gif")?.entry).toBe("yamada_pixel/gifs/walk_left.gif");
    expect(pickWalk(clerk, "right", "yamada_pixel/gifs/smoke_sit.gif")?.entry).toBe("yamada_pixel/gifs/walk_right.gif");
  });

  it("只有一个方向的就镜像过来；不分方向的往哪边拖都用它", () => {
    const onlyRight = ["idle.gif", "walk_right.gif"];
    expect(pickWalk(onlyRight, "left", "idle.gif")).toEqual({ entry: "walk_right.gif", flip: true });
    expect(pickWalk(onlyRight, "right", "idle.gif")).toEqual({ entry: "walk_right.gif", flip: false });
    expect(pickWalk(clawd, "left", clawd[0])).toEqual({ entry: "clawd_gifs/clawd-crabwalk.gif", flip: false });
    expect(pickWalk(clawd, "right", clawd[0])).toEqual({ entry: "clawd_gifs/clawd-crabwalk.gif", flip: false });
  });

  it("没有走路动作的包，拖动时不换动作", () => {
    expect(pickWalk(nashorV1, "left", nashorV1[0])).toBeNull();
  });
});

describe("拖动方向", () => {
  it("往哪边拖就朝哪边走；主要是上下拖的不算", () => {
    expect(nextDragWalk(STANDING, -6, 1)).toEqual({ direction: "left", backtrack: 0 });
    expect(nextDragWalk(STANDING, 3, 0)).toEqual({ direction: "right", backtrack: 0 });
    expect(nextDragWalk(STANDING, 0, 12)).toBeNull();
    expect(nextDragWalk(STANDING, 2, 10)).toBeNull();
  });

  it("往回抖一两个像素不转身，往回拖够了才转", () => {
    const left = { direction: "left" as const, backtrack: 0 };
    const jitter = nextDragWalk(left, 2, 0);
    expect(jitter).toEqual({ direction: "left", backtrack: 2 });
    expect(nextDragWalk(jitter!, 3, 0)).toEqual({ direction: "right", backtrack: 0 });
    // 抖完继续往左，累计清零
    expect(nextDragWalk(jitter!, -5, 0)).toEqual({ direction: "left", backtrack: 0 });
  });
});
