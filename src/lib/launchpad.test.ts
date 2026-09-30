import { describe, expect, it } from "vitest";
import { addToFolder, groupApps, moveItem, renameFolder, serializeLayout, takeOut, type LaunchItem } from "./launchpad";

const apps: LaunchItem[] = [
  { kind: "app", id: "a", name: "甲" },
  { kind: "app", id: "b", name: "乙" },
  { kind: "app", id: "c", name: "丙" },
];

describe("启动台文件夹", () => {
  it("拖到另一个应用上，在那个位置收成文件夹", () => {
    const next = groupApps(apps, "c", "a", "f1");
    expect(serializeLayout(next)).toEqual({
      order: ["folder:f1", "app:b"],
      folders: [{ id: "f1", name: "未命名", appIds: ["a", "c"] }],
    });
  });

  it("再拖一个进去，原来的格子上就不再单列它", () => {
    const folder = groupApps(apps, "b", "a", "f1");
    const next = addToFolder(folder, "c", "f1");
    expect(next).toEqual([
      { kind: "folder", id: "f1", name: "未命名", apps: [
        { id: "a", name: "甲" },
        { id: "b", name: "乙" },
        { id: "c", name: "丙" },
      ] },
    ]);
  });

  it("拖出来后放在文件夹后面，只剩一个就解散", () => {
    const folder = groupApps(apps, "b", "a", "f1");
    const taken = takeOut(folder, "f1", "b");
    expect(taken.map((item) => item.id)).toEqual(["a", "b", "c"]);
    expect(taken.every((item) => item.kind === "app")).toBe(true);
  });

  it("文件夹可以改名，空名字回到未命名", () => {
    const folder = groupApps(apps, "b", "a", "f1");
    expect(renameFolder(folder, "f1", "  工作  ")[0]).toMatchObject({ name: "工作" });
    expect(renameFolder(folder, "f1", "   ")[0]).toMatchObject({ name: "未命名" });
  });

  it("拖到别的格子上是换位置，不是收成文件夹", () => {
    expect(moveItem(apps, "a", 2).map((item) => item.id)).toEqual(["b", "a", "c"]);
  });
});
