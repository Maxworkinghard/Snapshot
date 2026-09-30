/** 启动台格子上的一项。文件夹里的应用不再出现在外层。 */

export interface LaunchApp {
  id: string;
  name: string;
}

export type LaunchItem =
  | ({ kind: "app" } & LaunchApp)
  | { kind: "folder"; id: string; name: string; apps: LaunchApp[] };

export interface LaunchpadView {
  apps: LaunchApp[];
  items: LaunchItem[];
}

export const LAUNCHPAD_COLUMNS = 7;
export const UNNAMED_FOLDER = "未命名";

export function pageCount(itemCount: number, pageSize: number) {
  return Math.max(1, Math.ceil(itemCount / Math.max(1, pageSize)));
}

function findApp(items: LaunchItem[], id: string): LaunchApp | null {
  for (const item of items) {
    if (item.kind === "app" && item.id === id) return { id: item.id, name: item.name };
    if (item.kind === "folder") {
      const found = item.apps.find((app) => app.id === id);
      if (found) return found;
    }
  }
  return null;
}

/** 拿走一个应用。文件夹因此少于两个时，剩下的那个回到格子上。 */
function withoutApp(items: LaunchItem[], id: string): LaunchItem[] {
  const next: LaunchItem[] = [];
  for (const item of items) {
    if (item.kind === "app") {
      if (item.id !== id) next.push(item);
      continue;
    }
    const apps = item.apps.filter((app) => app.id !== id);
    if (apps.length >= 2) next.push({ ...item, apps });
    else if (apps.length === 1) next.push({ kind: "app", ...apps[0] });
  }
  return next;
}

/** 把一个应用拖到另一个应用上，在目标原来的位置收成文件夹。 */
export function groupApps(items: LaunchItem[], sourceId: string, targetId: string, folderId: string): LaunchItem[] {
  if (sourceId === targetId) return items;
  const source = findApp(items, sourceId);
  if (!source || !items.some((item) => item.kind === "app" && item.id === targetId)) return items;
  const removed = withoutApp(items, sourceId);
  const index = removed.findIndex((item) => item.kind === "app" && item.id === targetId);
  if (index < 0) return items;
  const target = removed[index];
  if (target.kind !== "app") return items;
  const next = removed.slice();
  next.splice(index, 1, {
    kind: "folder",
    id: folderId,
    name: UNNAMED_FOLDER,
    apps: [
      { id: target.id, name: target.name },
      source,
    ],
  });
  return next;
}

export function addToFolder(items: LaunchItem[], sourceId: string, folderId: string): LaunchItem[] {
  const source = findApp(items, sourceId);
  const folder = items.find((item) => item.kind === "folder" && item.id === folderId);
  if (!source || !folder || folder.kind !== "folder" || folder.apps.some((app) => app.id === sourceId)) return items;
  const removed = withoutApp(items, sourceId);
  return removed.map((item) =>
    item.kind === "folder" && item.id === folderId ? { ...item, apps: [...item.apps, source] } : item,
  );
}

export function moveItem(items: LaunchItem[], id: string, toIndex: number): LaunchItem[] {
  const from = items.findIndex((item) => item.id === id);
  if (from < 0 || from === toIndex) return items;
  const next = items.slice();
  const [item] = next.splice(from, 1);
  const dest = Math.max(0, Math.min(toIndex > from ? toIndex - 1 : toIndex, next.length));
  next.splice(dest, 0, item);
  return next;
}

export function reorderInFolder(items: LaunchItem[], folderId: string, appId: string, toIndex: number): LaunchItem[] {
  let changed = false;
  const next = items.map((item) => {
    if (item.kind !== "folder" || item.id !== folderId) return item;
    const from = item.apps.findIndex((app) => app.id === appId);
    if (from < 0 || from === toIndex) return item;
    const apps = item.apps.slice();
    const [app] = apps.splice(from, 1);
    apps.splice(Math.max(0, Math.min(toIndex > from ? toIndex - 1 : toIndex, apps.length)), 0, app);
    changed = true;
    return { ...item, apps };
  });
  return changed ? next : items;
}

/** 从文件夹里拖出来，放在文件夹后面。只剩一个时文件夹消失。 */
export function takeOut(items: LaunchItem[], folderId: string, appId: string): LaunchItem[] {
  const index = items.findIndex((item) => item.kind === "folder" && item.id === folderId);
  if (index < 0) return items;
  const folder = items[index];
  if (folder.kind !== "folder") return items;
  const app = folder.apps.find((item) => item.id === appId);
  if (!app) return items;
  const apps = folder.apps.filter((item) => item.id !== appId);
  const next = items.slice();
  if (apps.length >= 2) next[index] = { ...folder, apps };
  else if (apps.length === 1) next[index] = { kind: "app", ...apps[0] };
  else next.splice(index, 1);
  next.splice(Math.min(index + 1, next.length), 0, { kind: "app", ...app });
  return next;
}

export function renameFolder(items: LaunchItem[], folderId: string, name: string): LaunchItem[] {
  const trimmed = name.trim().slice(0, 24) || UNNAMED_FOLDER;
  let changed = false;
  const next = items.map((item) => {
    if (item.kind !== "folder" || item.id !== folderId || item.name === trimmed) return item;
    changed = true;
    return { ...item, name: trimmed };
  });
  return changed ? next : items;
}

export function serializeLayout(items: LaunchItem[]) {
  return {
    order: items.map((item) => (item.kind === "app" ? `app:${item.id}` : `folder:${item.id}`)),
    folders: items.flatMap((item) =>
      item.kind === "folder" ? [{ id: item.id, name: item.name, appIds: item.apps.map((app) => app.id) }] : [],
    ),
  };
}
