import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { hideLaunchpad, launchApp, launchpadState, onLaunchpadOpened, saveLaunchpad } from "../lib/backend";
import {
  addToFolder,
  groupApps,
  LAUNCHPAD_COLUMNS,
  moveItem,
  pageCount,
  renameFolder,
  reorderInFolder,
  serializeLayout,
  takeOut,
  type LaunchApp,
  type LaunchItem,
} from "../lib/launchpad";
import {
  clampSpringVelocity,
  FLICK_VELOCITY,
  gestureVelocity,
  springSettled,
  stepSpring,
  targetPage,
  trackX,
  emptyWheelDetents,
  noteWheelDetent,
  wheelEase,
  WHEEL_CLUSTER_MS,
  WHEEL_EASE_MS,
  WHEEL_QUIET_MS,
  type Sample,
  type Spring,
} from "../lib/launchpadPaging";
import { launchIconUrl } from "../lib/media";
import { DURATION, motionReduced } from "../lib/motion";

type Hit = {
  type: "icon" | "tile" | "out";
  kind: "app" | "folder";
  id: string;
  index: number;
  folderId: string | null;
};

type Drag = {
  kind: "app" | "folder";
  id: string;
  name: string;
  fromFolder: string | null;
  x: number;
  y: number;
  overId: string | null;
};

type TileInfo = { kind: "app" | "folder"; id: string; name: string; fromFolder: string | null };

type Pan = {
  visual: number;
  width: number;
  springing: boolean;
  tracking: boolean;
  paused: boolean;
  raf: number;
  wheelTimer: number;
  wheeling: boolean;
  wheelOrigin: number;
  wheelBase: number;
  wheelDrag: number;
  wheelDetents: ReturnType<typeof emptyWheelDetents>;
  stopPointer: (() => void) | null;
};

const PAN_SLOP = 8;

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

function readWidth(viewport: HTMLElement | null, pan: Pan) {
  const measured = viewport?.clientWidth ?? 0;
  const next = measured > 0 ? measured : window.innerWidth || pan.width || 1;
  pan.width = next;
  if (viewport) viewport.style.setProperty("--lp-page", `${next}px`);
  return next;
}

export function LaunchpadWindow() {
  const [apps, setApps] = useState<LaunchApp[]>([]);
  const [items, setItems] = useState<LaunchItem[]>([]);
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(0);
  const [rows, setRows] = useState(5);
  const [openFolder, setOpenFolder] = useState<string | null>(null);
  const [editingName, setEditingName] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [drag, setDrag] = useState<Drag | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const itemsRef = useRef(items);
  const openFolderRef = useRef(openFolder);
  const pageRef = useRef(0);
  const pageGate = useRef({ pages: 1, hold: false });
  const sheetRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const viewportRef = useRef<HTMLDivElement>(null);
  const trackRef = useRef<HTMLDivElement>(null);
  const pan = useRef<Pan>({
    visual: 0,
    width: 0,
    springing: false,
    tracking: false,
    paused: false,
    raf: 0,
    wheelTimer: 0,
    wheeling: false,
    wheelOrigin: 0,
    wheelBase: 0,
    wheelDrag: 0,
    wheelDetents: emptyWheelDetents(),
    stopPointer: null,
  });
  const motion = useRef({
    readWidth: (): number => 1,
    writeX: (_x: number) => {},
    release: (_origin: number, _drag: number, _velocity: number) => {},
    settle: () => {},
    goTo: (_index: number) => {},
    glideTo: (_index: number) => {},
  });
  itemsRef.current = items;
  openFolderRef.current = openFolder;

  const pageSize = LAUNCHPAD_COLUMNS * rows;
  const pages = pageCount(items.length, pageSize);
  const safePage = Math.min(page, Math.max(0, pages - 1));
  pageRef.current = safePage;

  function writeX(x: number) {
    pan.current.visual = x;
    const node = trackRef.current;
    if (node) node.style.transform = `translate3d(${x}px, 0, 0)`;
  }

  function cancelRaf() {
    if (pan.current.raf) cancelAnimationFrame(pan.current.raf);
    pan.current.raf = 0;
    pan.current.springing = false;
  }

  function resetMotion() {
    pan.current.stopPointer?.();
    pan.current.stopPointer = null;
    cancelRaf();
    window.clearTimeout(pan.current.wheelTimer);
    pan.current.tracking = false;
    pan.current.paused = false;
    pan.current.wheeling = false;
    pan.current.wheelDetents = emptyWheelDetents();
    pageRef.current = 0;
    writeX(0);
  }

  function startSpring(from: number, velocity: number, target: number, width: number) {
    cancelRaf();
    const to = -target * width;
    pageRef.current = target;
    setPage(target);
    const travel = to - from;
    if (motionReduced() || Math.abs(travel) < 0.6) {
      pan.current.springing = false;
      pan.current.tracking = false;
      writeX(to);
      return;
    }
    let speed = clampSpringVelocity(velocity);
    if (travel * speed < 0) speed = 0;
    pan.current.springing = true;
    pan.current.tracking = false;
    let state: Spring = { x: from, v: speed, target: to };
    let last = performance.now();
    const tick = (now: number) => {
      if (!pan.current.springing) return;
      const dt = Math.min(32, now - last);
      last = now;
      if (dt > 0) state = stepSpring(state, dt);
      if (dt > 0 && springSettled(state)) {
        const settledWidth = readWidth(viewportRef.current, pan.current);
        writeX(-pageRef.current * settledWidth);
        pan.current.springing = false;
        pan.current.raf = 0;
        return;
      }
      if (dt > 0) writeX(state.x);
      pan.current.raf = requestAnimationFrame(tick);
    };
    pan.current.raf = requestAnimationFrame(tick);
  }

  function release(origin: number, drag: number, velocity: number) {
    const width = readWidth(viewportRef.current, pan.current);
    const target = targetPage(origin, pageGate.current.pages, drag, velocity, width);
    pan.current.tracking = false;
    pan.current.paused = false;
    pan.current.wheeling = false;
    startSpring(pan.current.visual, velocity, target, width);
  }

  function settle() {
    const width = readWidth(viewportRef.current, pan.current);
    pan.current.tracking = false;
    pan.current.paused = false;
    pan.current.wheeling = false;
    startSpring(pan.current.visual, 0, pageRef.current, width);
  }

  function goTo(index: number) {
    window.clearTimeout(pan.current.wheelTimer);
    pan.current.wheeling = false;
    const width = readWidth(viewportRef.current, pan.current);
    const max = Math.max(0, pageGate.current.pages - 1);
    const target = Math.min(max, Math.max(0, index));
    pan.current.tracking = false;
    pan.current.paused = false;
    startSpring(pan.current.visual, 0, target, width);
  }

  function glideTo(index: number) {
    const width = readWidth(viewportRef.current, pan.current);
    const max = Math.max(0, pageGate.current.pages - 1);
    const target = Math.min(max, Math.max(0, index));
    cancelRaf();
    const from = pan.current.visual;
    const to = -target * width;
    pan.current.paused = false;
    pan.current.springing = true;
    pageRef.current = target;
    setPage(target);
    if (motionReduced() || Math.abs(to - from) < 0.6) {
      pan.current.springing = false;
      writeX(to);
      return;
    }
    const started = performance.now();
    const tick = (now: number) => {
      if (!pan.current.springing) return;
      const elapsed = now - started;
      if (elapsed >= WHEEL_EASE_MS) {
        const settledWidth = readWidth(viewportRef.current, pan.current);
        writeX(-pageRef.current * settledWidth);
        pan.current.springing = false;
        pan.current.raf = 0;
        return;
      }
      writeX(wheelEase(from, to, elapsed, WHEEL_EASE_MS));
      pan.current.raf = requestAnimationFrame(tick);
    };
    pan.current.raf = requestAnimationFrame(tick);
  }

  motion.current.readWidth = () => readWidth(viewportRef.current, pan.current);
  motion.current.writeX = writeX;
  motion.current.release = release;
  motion.current.settle = settle;
  motion.current.goTo = goTo;
  motion.current.glideTo = glideTo;

  useEffect(() => {
    const measure = () => {
      const available = window.innerHeight - 180;
      setRows(Math.max(3, Math.min(5, Math.floor(available / 132))));
      if (pan.current.springing || pan.current.tracking || !viewportRef.current) return;
      writeX(-pageRef.current * readWidth(viewportRef.current, pan.current));
    };
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, []);

  /**
   * 淡入淡出都从这里走。
   * 必须先撤掉上一次的动画：退场用了 fill: forwards 把元素按在 opacity 0 上，
   * 新动画播完（fill: none）之后那次填充会重新生效，第二次打开就会是整块透明。
   */
  const fade = useRef<Animation | null>(null);
  const playFade = (from: number, to: number, ms: number, easing: string) => {
    const root = rootRef.current;
    if (!root || typeof root.animate !== "function") return null;
    fade.current?.cancel();
    const animation = root.animate([{ opacity: from }, { opacity: to }], { duration: ms, easing, fill: "forwards" });
    fade.current = animation;
    return animation;
  };

  /**
   * 打开时整块淡入。窗口是提前建好、直接 show() 出来的，不做这层就是硬切。
   * 顺手把还没挂完图标的那一帧也盖住：空格子的淡入比看着图标一个个跳出来干净。
   */
  const fadeIn = () => {
    playFade(0, 1, motionReduced() ? DURATION.fade : DURATION.open, "cubic-bezier(0.2, 0, 0, 1)");
  };

  /** 关闭进行中。它同时是「这次关闭还算数吗」的标记：重开时会被清掉 */
  const closing = useRef(false);

  /**
   * 关闭时先淡出，动画播完再让后端收起窗口。窗口是整块盖住屏幕的，
   * 直接 hide() 就是硬切一下没了；先淡出去才对得上打开时的淡入。
   * 播不了动画（测试环境、旧 WebView）就立刻收起，不能把面板卡在屏幕上。
   */
  const close = () => {
    if (closing.current) return;
    closing.current = true;
    const animation = playFade(1, 0, motionReduced() ? DURATION.fade : DURATION.exit, "cubic-bezier(0.3, 0, 0.8, 0.15)");
    const done = () => {
      // 淡出途中又被打开（连点桌宠）时，这次关闭已经不算数了，不能把刚开的窗口收掉
      if (closing.current) void hideLaunchpad();
    };
    if (!animation) {
      done();
      return;
    }
    // 正常播完和被打断都会走到 finished，两条路都要收尾
    void animation.finished.then(done, done);
  };

  useEffect(() => {
    let alive = true;
    const reload = () => {
      resetMotion();
      setQuery("");
      setPage(0);
      setOpenFolder(null);
      setEditingName(false);
      setError(null);
      setLoading(true);
      closing.current = false;
      fadeIn();
      void launchpadState()
        .then((state) => {
          if (!alive) return;
          setApps(state.apps);
          setItems(state.items);
        })
        .catch((reason) => {
          if (alive) setError(errorText(reason));
        })
        .finally(() => {
          if (alive) setLoading(false);
          searchRef.current?.focus();
        });
    };
    let unlisten: (() => void) | undefined;
    void onLaunchpadOpened(reload).then((stop) => {
      if (!alive) {
        stop();
        return;
      }
      unlisten = stop;
    });
    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (query) {
        setQuery("");
        return;
      }
      if (openFolderRef.current) {
        setOpenFolder(null);
        setEditingName(false);
        return;
      }
      close();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [query]);

  useEffect(() => {
    const api = motion.current;
    const state = pan.current;
    const stopWheel = () => {
      window.clearTimeout(state.wheelTimer);
      state.wheeling = false;
    };
    const canPage = () => !pageGate.current.hold && pageGate.current.pages > 1;
    const onWheel = (event: WheelEvent) => {
      if (event.ctrlKey || !canPage()) return;
      event.preventDefault();
      const dominant = Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY;
      if (dominant === 0) return;
      const now = performance.now();
      // 凑满两格后，同一格里剩下的上报不能另开手势，否则会把滑动掐掉。
      if (!state.wheeling && state.springing && state.wheelDetents.armed && now - state.wheelDetents.at <= WHEEL_CLUSTER_MS) {
        return;
      }
      const width = api.readWidth();
      if (!state.wheeling) {
        if (state.raf) cancelAnimationFrame(state.raf);
        state.raf = 0;
        state.springing = false;
        state.wheeling = true;
        state.tracking = true;
        state.paused = false;
        state.wheelOrigin = pageRef.current;
        state.wheelBase = state.visual + pageRef.current * width;
        state.wheelDrag = state.wheelBase;
        state.wheelDetents = emptyWheelDetents();
      }
      const previous = state.wheelDetents.count;
      state.wheelDetents = noteWheelDetent(state.wheelDetents, dominant, now);
      const count = state.wheelDetents.count;
      if (count !== previous) {
        // 一格一页。以前要两格才翻，加上停手 400ms 就重置计数，
        // 正常速度滚一两格（间隔常是 300~800ms）永远翻不了页。
        api.glideTo(state.wheelOrigin + count);
      }
      window.clearTimeout(state.wheelTimer);
      state.wheelTimer = window.setTimeout(() => {
        if (!state.wheeling) return;
        state.wheeling = false;
        state.tracking = false;
      }, WHEEL_QUIET_MS);
    };
    const swallowSideButton = (event: MouseEvent) => {
      if (event.button === 3 || event.button === 4) event.preventDefault();
    };
    const onSideButton = (event: PointerEvent) => {
      if (event.button !== 3 && event.button !== 4) return;
      event.preventDefault();
      if (!canPage()) return;
      stopWheel();
      const direction = event.button === 4 ? 1 : -1;
      const velocity = direction > 0 ? -(FLICK_VELOCITY + 0.05) : FLICK_VELOCITY + 0.05;
      api.release(pageRef.current, 0, velocity);
    };
    window.addEventListener("wheel", onWheel, { passive: false });
    window.addEventListener("pointerdown", swallowSideButton);
    window.addEventListener("auxclick", swallowSideButton);
    window.addEventListener("pointerup", onSideButton);
    return () => {
      stopWheel();
      if (state.raf) cancelAnimationFrame(state.raf);
      state.stopPointer?.();
      window.removeEventListener("wheel", onWheel);
      window.removeEventListener("pointerdown", swallowSideButton);
      window.removeEventListener("auxclick", swallowSideButton);
      window.removeEventListener("pointerup", onSideButton);
    };
  }, []);

  useLayoutEffect(() => {
    const width = readWidth(viewportRef.current, pan.current);
    if (pan.current.springing || pan.current.tracking) {
      writeX(pan.current.visual);
      return;
    }
    writeX(-pageRef.current * width);
  }, [page, pages, query, items.length, rows]);

  const needle = query.trim().toLowerCase();
  const results = needle ? apps.filter((app) => app.name.toLowerCase().includes(needle)) : null;
  const folder = items.find((item) => item.kind === "folder" && item.id === openFolder);
  pageGate.current = {
    pages,
    hold: Boolean(needle) || openFolder !== null || drag !== null,
  };

  function commit(next: LaunchItem[]) {
    setItems(next);
    if (openFolderRef.current && !next.some((item) => item.kind === "folder" && item.id === openFolderRef.current)) {
      setOpenFolder(null);
    }
    void saveLaunchpad(serializeLayout(next)).catch((reason) => setError(errorText(reason)));
  }

  function hitAt(x: number, y: number): Hit | null {
    const element = typeof document.elementFromPoint === "function" ? document.elementFromPoint(x, y) : null;
    if (!element) return null;
    const sheet = sheetRef.current;
    if (sheet && !sheet.contains(element)) return { type: "out", kind: "app", id: "", index: 0, folderId: null };
    const tile = element.closest<HTMLElement>(".launchpad-tile");
    if (!tile?.dataset.id) return null;
    return {
      type: element.closest("[data-icon]") ? "icon" : "tile",
      kind: tile.dataset.kind === "folder" ? "folder" : "app",
      id: tile.dataset.id,
      index: Number(tile.dataset.index ?? "0"),
      folderId: tile.dataset.folder || null,
    };
  }

  function dropped(current: Drag, hit: Hit | null): LaunchItem[] | null {
    const list = itemsRef.current;
    if (!hit || hit.type === "out") {
      if (current.kind === "app" && current.fromFolder) return takeOut(list, current.fromFolder, current.id);
      return null;
    }
    if (current.kind === "folder") {
      if (hit.folderId || hit.id === current.id) return null;
      return moveItem(list, current.id, hit.index);
    }
    if (hit.id === current.id) return null;
    if (hit.type === "icon" && hit.kind === "app" && !hit.folderId) {
      return groupApps(list, current.id, hit.id, `fld-${crypto.randomUUID()}`);
    }
    if (hit.kind === "folder" && (hit.type === "icon" || hit.folderId === null)) {
      return addToFolder(list, current.id, hit.id);
    }
    if (hit.folderId && hit.folderId === current.fromFolder) return reorderInFolder(list, hit.folderId, current.id, hit.index);
    if (hit.folderId) return addToFolder(list, current.id, hit.folderId);
    if (!current.fromFolder) return moveItem(list, current.id, hit.index);
    return null;
  }

  function beginPointer(startX: number, startY: number, source: { kind: "gap" } | { kind: "tile"; tile: TileInfo }) {
    const state = pan.current;
    state.stopPointer?.();
    const width = motion.current.readWidth();
    const origin = pageRef.current;
    const interrupted = state.springing || state.wheeling;
    if (state.raf) cancelAnimationFrame(state.raf);
    state.raf = 0;
    state.springing = false;
    window.clearTimeout(state.wheelTimer);
    state.wheeling = false;
    state.tracking = true;
    state.paused = interrupted;
    const grabDrag = state.visual + origin * width;
    let mode: "pending" | "pan" | "icon" | "shift" = "pending";
    const samples: Sample[] = [{ t: performance.now(), x: grabDrag }];
    const fromFolder = source.kind === "tile" ? source.tile.fromFolder : null;

    const move = (pointer: PointerEvent) => {
      const dx = pointer.clientX - startX;
      const dy = pointer.clientY - startY;
      if (mode === "pending") {
        if (Math.hypot(dx, dy) < PAN_SLOP) return;
        const allowPan = !fromFolder && !pageGate.current.hold && pageGate.current.pages > 1 && Math.abs(dx) > Math.abs(dy);
        if (allowPan) mode = "pan";
        else if (source.kind === "tile") mode = "icon";
        else mode = "shift";
      }
      if (mode === "pan") {
        const drag = grabDrag + dx;
        samples.push({ t: performance.now(), x: drag });
        motion.current.writeX(trackX(origin, pageGate.current.pages, drag, width));
        return;
      }
      if (mode === "icon" && source.kind === "tile") {
        const tile = source.tile;
        const hit = hitAt(pointer.clientX, pointer.clientY);
        setDrag({
          kind: tile.kind,
          id: tile.id,
          name: tile.name,
          fromFolder: tile.fromFolder,
          x: pointer.clientX,
          y: pointer.clientY,
          overId: hit && hit.id !== tile.id ? hit.id : null,
        });
      }
    };

    const end = (pointer: PointerEvent) => {
      if (pointer.type !== "pointercancel" && pointer.button !== 0) return;
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      if (state.stopPointer === stop) state.stopPointer = null;
      if (mode === "pan") {
        const dx = pointer.clientX - startX;
        samples.push({ t: performance.now(), x: grabDrag + dx });
        motion.current.release(origin, dx, gestureVelocity(samples, performance.now()));
        return;
      }
      state.tracking = false;
      if (mode === "icon" && source.kind === "tile") {
        const tile = source.tile;
        const current: Drag = {
          kind: tile.kind,
          id: tile.id,
          name: tile.name,
          fromFolder: tile.fromFolder,
          x: pointer.clientX,
          y: pointer.clientY,
          overId: null,
        };
        const next = dropped(current, hitAt(pointer.clientX, pointer.clientY));
        setDrag(null);
        if (next && next !== itemsRef.current) commit(next);
        if (state.paused) motion.current.settle();
        return;
      }
      if (mode === "pending" && source.kind === "tile") {
        const tile = source.tile;
        if (tile.kind === "folder") {
          setOpenFolder(tile.id);
          setEditingName(false);
        } else {
          void runApp(tile.id);
        }
      } else if (mode === "pending") {
        if (openFolderRef.current) {
          setOpenFolder(null);
          setEditingName(false);
        } else {
          state.paused = false;
          close();
          return;
        }
      }
      if (state.paused) motion.current.settle();
    };

    const stop = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      if (state.stopPointer === stop) state.stopPointer = null;
    };
    state.stopPointer = stop;
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
  }

  function onTilePointerDown(event: React.PointerEvent<HTMLButtonElement>, tile: TileInfo) {
    if (event.button !== 0) return;
    if (results) {
      const up = () => {
        window.removeEventListener("pointerup", up);
        if (tile.kind === "app") void runApp(tile.id);
      };
      window.addEventListener("pointerup", up);
      return;
    }
    beginPointer(event.clientX, event.clientY, { kind: "tile", tile });
  }

  function onRootPointerDown(event: React.PointerEvent<HTMLDivElement>) {
    if (event.button !== 0) return;
    const target = event.target as HTMLElement;
    if (target.closest(".launchpad-tile, .launchpad-search, .launchpad-folder, .launchpad-dots")) return;
    beginPointer(event.clientX, event.clientY, { kind: "gap" });
  }

  async function runApp(id: string) {
    // 启动成功时后端会把面板收起来，先自己淡出，别让整个屏幕闪一下才消失。
    // 失败要留在原地报错，所以只有成功才让它保持透明。
    try {
      await launchApp(id);
      playFade(1, 0, motionReduced() ? DURATION.fade : DURATION.exit, "cubic-bezier(0.3, 0, 0.8, 0.15)");
    } catch (reason) {
      setError(errorText(reason));
    }
  }

  function appTile(app: LaunchApp, index: number) {
    return (
      <Tile
        key={app.id}
        kind="app"
        id={app.id}
        name={app.name}
        index={index}
        marked={drag?.overId === app.id}
        onPointerDown={(event) => onTilePointerDown(event, { kind: "app", id: app.id, name: app.name, fromFolder: null })}
      >
        <span className="launchpad-icon" data-icon="">
          <AppIcon id={app.id} name={app.name} size={88} />
        </span>
      </Tile>
    );
  }

  function pageTiles(list: LaunchItem[], start: number) {
    return list.map((item, index) => {
      const absolute = start + index;
      if (item.kind === "folder") {
        return (
          <Tile
            key={item.id}
            kind="folder"
            id={item.id}
            name={item.name}
            index={absolute}
            marked={drag?.overId === item.id}
            onPointerDown={(event) =>
              onTilePointerDown(event, { kind: "folder", id: item.id, name: item.name, fromFolder: null })
            }
          >
            <span className="launchpad-folder-preview" data-icon="">
              {item.apps.slice(0, 4).map((app) => (
                <AppIcon key={app.id} id={app.id} name={app.name} size={28} />
              ))}
            </span>
          </Tile>
        );
      }
      return appTile(item, absolute);
    });
  }

  const shownCount = results ? results.length : items.length;

  return (
    <div className="launchpad" ref={rootRef} onPointerDown={onRootPointerDown}>
      <input
        ref={searchRef}
        className="launchpad-search"
        placeholder="搜索应用"
        value={query}
        aria-label="搜索应用"
        onChange={(event) => {
          setQuery(event.target.value);
          setOpenFolder(null);
        }}
        onMouseDown={(event) => event.stopPropagation()}
      />
      {loading && items.length === 0 ? <p className="launchpad-status">正在读取本机应用…</p> : null}
      {!loading && shownCount === 0 ? (
        <p className="launchpad-status">{needle ? "没有匹配的应用" : "没有找到可启动的应用"}</p>
      ) : results ? (
        <div className="launchpad-grid is-search">{results.map((app, index) => appTile(app, index))}</div>
      ) : (
        <div className="launchpad-viewport" ref={viewportRef}>
          <div className="launchpad-track" ref={trackRef}>
            {Array.from({ length: pages }, (_, index) => (
              <div key={index} className="launchpad-page" aria-hidden={index === safePage ? undefined : true}>
                <div className="launchpad-grid">
                  {pageTiles(items.slice(index * pageSize, index * pageSize + pageSize), index * pageSize)}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
      {!results && pages > 1 ? (
        <div className="launchpad-dots" role="tablist" aria-label="启动台分页">
          {Array.from({ length: pages }, (_, index) => (
            <button
              key={index}
              type="button"
              className={index === safePage ? "is-current" : ""}
              aria-label={`第 ${index + 1} 页`}
              onMouseDown={(event) => event.stopPropagation()}
              onClick={() => motion.current.goTo(index)}
            />
          ))}
        </div>
      ) : null}
      {error ? <p className="launchpad-status is-error">{error}</p> : null}
      {folder && folder.kind === "folder" && !results ? (
        <div className="launchpad-folder-layer">
          <div className="launchpad-folder" ref={sheetRef} onMouseDown={(event) => event.stopPropagation()}>
            {editingName ? (
              <input
                className="launchpad-folder-name"
                aria-label="文件夹名称"
                autoFocus
                defaultValue={folder.name}
                onBlur={(event) => {
                  commit(renameFolder(itemsRef.current, folder.id, event.target.value));
                  setEditingName(false);
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter") event.currentTarget.blur();
                }}
              />
            ) : (
              <button type="button" className="launchpad-folder-name" onClick={() => setEditingName(true)}>
                {folder.name}
              </button>
            )}
            <div className="launchpad-folder-apps">
              {folder.apps.map((app, index) => (
                <Tile
                  key={app.id}
                  kind="app"
                  id={app.id}
                  name={app.name}
                  index={index}
                  folderId={folder.id}
                  marked={drag?.overId === app.id}
                  onPointerDown={(event) =>
                    onTilePointerDown(event, { kind: "app", id: app.id, name: app.name, fromFolder: folder.id })
                  }
                >
                  <span className="launchpad-icon" data-icon="">
                    <AppIcon id={app.id} name={app.name} size={72} />
                  </span>
                </Tile>
              ))}
            </div>
          </div>
        </div>
      ) : null}
      {drag ? (
        <div className="launchpad-ghost" style={{ left: drag.x, top: drag.y }}>
          <AppIcon id={drag.kind === "app" ? drag.id : ""} name={drag.name} size={72} />
        </div>
      ) : null}
    </div>
  );
}

function Tile({
  kind,
  id,
  name,
  index,
  folderId,
  marked,
  onPointerDown,
  children,
}: {
  kind: "app" | "folder";
  id: string;
  name: string;
  index: number;
  folderId?: string;
  marked: boolean;
  onPointerDown: (event: React.PointerEvent<HTMLButtonElement>) => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      className={`launchpad-tile ${marked ? "is-drop" : ""}`}
      data-kind={kind}
      data-id={id}
      data-index={index}
      data-folder={folderId ?? ""}
      aria-label={name}
      onPointerDown={onPointerDown}
    >
      {children}
      <span className="launchpad-label">{name}</span>
    </button>
  );
}

function AppIcon({ id, name, size }: { id: string; name: string; size: number }) {
  const [failed, setFailed] = useState(false);
  if (!id || failed) return <span className="launchpad-fallback">{Array.from(name)[0] ?? "?"}</span>;
  return <img src={launchIconUrl(id, size)} alt="" draggable={false} onError={() => setFailed(true)} />;
}
