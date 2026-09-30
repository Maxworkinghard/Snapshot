import { useEffect, useRef, useState } from "react";
import { currentMonitor, getCurrentWindow, LogicalSize, PhysicalPosition, type Window } from "@tauri-apps/api/window";
import { AppWindow, PawPrint } from "lucide-react";
import {
  getPreviousApp,
  loadSettings,
  onPreviousAppChanged,
  onSettingsChanged,
  showLaunchpad,
  showQuickMenu,
} from "../lib/backend";
import { iconUrl, petUrl, petWalkUrl } from "../lib/media";
import { MediaImage } from "../components/MediaImage";
import { broadcastPetAnimation, onPetAnimationAsked } from "../lib/petSync";
import { PET_SCALE, petWindowSize } from "../lib/petSize";
import { cycleEntries, nextDragWalk, pickWalk, STANDING, type DragWalk, type WalkDirection } from "../lib/petWalk";
import type { PreviousApp, Settings } from "../types";

/** 放大后超出了所在屏幕的可用区域，就挪回来 */
async function keepOnScreen(petWindow: Window) {
  const monitor = await currentMonitor();
  if (!monitor) return;
  const [position, size] = await Promise.all([petWindow.outerPosition(), petWindow.outerSize()]);
  const { position: origin, size: area } = monitor.workArea;
  const x = Math.max(origin.x, Math.min(position.x, origin.x + area.width - size.width));
  const y = Math.max(origin.y, Math.min(position.y, origin.y + area.height - size.height));
  if (x !== position.x || y !== position.y) await petWindow.setPosition(new PhysicalPosition(x, y));
}

/** 窗口停下这么久没再动，就算松手了，换回原来的动作 */
const WALK_SETTLE_MS = 250;
/** 点一下换上的动作只停这么久，然后回到默认待机 */
const CLICK_ACTION_MS = 3000;
/** 等这么久还没来第二下，才算单击。第二下是双击，打开启动台，不换动作 */
export const CLICK_DELAY_MS = 400;

export function PetWindow() {
  const [app, setApp] = useState<PreviousApp>({ id: null, pid: null, name: "", title: "" });
  const [appearanceId, setAppearanceId] = useState("app-icon");
  const [animations, setAnimations] = useState<string[]>([]);
  const [restingEntry, setRestingEntry] = useState<string | null>(null);
  const [shownEntry, setShownEntry] = useState<string | null>(null);
  const [petSize, setPetSize] = useState(60);
  // 读到设置前不定大小，免得先按默认缩放一次再跳到用户设的大小
  const [petScale, setPetScale] = useState<number | null>(null);
  const [walking, setWalking] = useState<WalkDirection | null>(null);
  const dragged = useRef(false);
  // 点过之后下一次从这里接着换，回到待机也不清掉，免得每次点击都是同一个动作
  const actionCursor = useRef(0);
  const returnTimer = useRef<number | undefined>(undefined);
  const clickTimer = useRef<number | undefined>(undefined);
  const restingRef = useRef<string | null>(null);

  // 点击切换只在常规动作里转，走路动作留给拖动
  const cycle = cycleEntries(animations);
  const resting = restingEntry && cycle.includes(restingEntry) ? restingEntry : (cycle[0] ?? null);
  restingRef.current = resting;

  useEffect(
    () => () => {
      window.clearTimeout(returnTimer.current);
      window.clearTimeout(clickTimer.current);
    },
    [],
  );

  useEffect(() => {
    const applyAppearance = (settings: Settings) => {
      window.clearTimeout(returnTimer.current);
      window.clearTimeout(clickTimer.current);
      clickTimer.current = undefined;
      const id = settings.selectedAppearanceId;
      const asset = settings.petAssets.find((item) => item.id === id);
      const entries = asset?.animations ?? [];
      const actions = cycleEntries(entries);
      const nextResting = asset && actions.includes(asset.entry) ? asset.entry : (actions[0] ?? null);
      actionCursor.current = Math.max(0, actions.indexOf(nextResting ?? ""));
      setAppearanceId(id);
      setAnimations(entries);
      setRestingEntry(nextResting);
      setShownEntry(nextResting);
      setPetScale(settings.petScale ?? PET_SCALE.default);
    };
    void getPreviousApp().then(setApp);
    void loadSettings().then(applyAppearance);
    const appListener = onPreviousAppChanged(setApp);
    const settingsListener = onSettingsChanged(applyAppearance);
    return () => {
      void appListener.then((unlisten) => unlisten());
      void settingsListener.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    if (petScale === null) return;
    const petWindow = getCurrentWindow();
    let active = true;
    let lastSize = 0;

    const fitToScreen = async () => {
      const shortEdge = Math.min(window.screen.availWidth, window.screen.availHeight);
      const nextSize = petWindowSize(shortEdge, petScale);
      if (active) setPetSize(nextSize);
      if (nextSize === lastSize) return;
      lastSize = nextSize;
      await petWindow.setSize(new LogicalSize(nextSize, nextSize));
      // 只在大小变了时才挪：平常拖动桌宠不去抢它的位置
      await keepOnScreen(petWindow);
    };

    void fitToScreen();
    const movedListener = petWindow.onMoved(() => void fitToScreen());
    const scaleListener = petWindow.onScaleChanged(() => void fitToScreen());
    return () => {
      active = false;
      void movedListener.then((unlisten) => unlisten());
      void scaleListener.then((unlisten) => unlisten());
    };
  }, [petScale]);

  // 拖动时网页收不到指针事件（系统接管了窗口），只能看窗口往哪边移
  useEffect(() => {
    const petWindow = getCurrentWindow();
    let last: { x: number; y: number } | null = null;
    let state: DragWalk = STANDING;
    let settle: number | undefined;
    const movedListener = petWindow.onMoved(({ payload }) => {
      const previous = last;
      last = { x: payload.x, y: payload.y };
      if (!previous) return;
      const next = nextDragWalk(state, payload.x - previous.x, payload.y - previous.y);
      if (!next) return;
      state = next;
      setWalking(next.direction);
      window.clearTimeout(settle);
      settle = window.setTimeout(() => {
        state = STANDING;
        setWalking(null);
      }, WALK_SETTLE_MS);
    });
    return () => {
      window.clearTimeout(settle);
      void movedListener.then((unlisten) => unlisten());
    };
  }, []);

  function onPointerDown(event: React.PointerEvent) {
    if (event.button !== 0) return;
    dragged.current = false;
    const startX = event.clientX;
    const startY = event.clientY;
    const finishClick = () => {
      cleanup();
      if (dragged.current) return;
      if (clickTimer.current !== undefined) {
        window.clearTimeout(clickTimer.current);
        clickTimer.current = undefined;
        void showLaunchpad(event.screenX, event.screenY);
        return;
      }
      clickTimer.current = window.setTimeout(() => {
        clickTimer.current = undefined;
        switchOnClick();
      }, CLICK_DELAY_MS);
    };
    const markDrag = (move: PointerEvent) => {
      if (Math.abs(move.clientX - startX) + Math.abs(move.clientY - startY) <= 4) return;
      dragged.current = true;
      window.clearTimeout(clickTimer.current);
      clickTimer.current = undefined;
      cleanup();
      void getCurrentWindow().startDragging();
    };
    const cleanup = () => {
      window.removeEventListener("pointermove", markDrag);
      window.removeEventListener("pointerup", finishClick);
    };
    window.addEventListener("pointermove", markDrag);
    window.addEventListener("pointerup", finishClick);
  }

  // 平时停在默认待机。点一下换到下一个非走路动作，停几秒再回到待机。
  function switchOnClick() {
    const rest = restingRef.current;
    if (appearanceId === "app-icon" || !rest || cycle.length < 2) return;
    for (let step = 1; step <= cycle.length; step += 1) {
      const index = (actionCursor.current + step) % cycle.length;
      const entry = cycle[index];
      if (entry === rest) continue;
      actionCursor.current = index;
      setShownEntry(entry);
      window.clearTimeout(returnTimer.current);
      returnTimer.current = window.setTimeout(() => setShownEntry(restingRef.current), CLICK_ACTION_MS);
      return;
    }
  }

  // 换了动作就告诉主窗口，侧栏的猫跟着换；主窗口刚打开来问时也答一声（走路不算，侧栏不跟着走）
  const currentEntry = shownEntry && cycle.includes(shownEntry) ? shownEntry : resting;
  const current = useRef({ assetId: appearanceId, entry: currentEntry });
  current.current = { assetId: appearanceId, entry: currentEntry };
  useEffect(() => {
    if (appearanceId !== "app-icon") broadcastPetAnimation({ assetId: appearanceId, entry: currentEntry });
  }, [appearanceId, currentEntry]);
  useEffect(
    () =>
      onPetAnimationAsked(() => {
        if (current.current.assetId !== "app-icon") broadcastPetAnimation(current.current);
      }),
    [],
  );

  const custom = appearanceId !== "app-icon";
  const petSrc = custom ? petUrl(appearanceId, currentEntry) : null;
  // 左右两张走路图一直挂着（藏起来），拖起来直接露出，不用临时取图
  const walks = custom
    ? (["left", "right"] as const).map((direction) => ({ direction, choice: pickWalk(animations, direction, currentEntry) }))
    : [];
  const shownWalk = walks.find((walk) => walk.direction === walking && walk.choice)?.direction ?? null;

  function onContextMenu(event: React.MouseEvent) {
    event.preventDefault();
    void showQuickMenu(event.screenX, event.screenY);
  }

  return (
    <div
      className={`pet-window ${custom ? "custom" : "app-icon"}`}
      onPointerDown={onPointerDown}
      onContextMenu={onContextMenu}
      title={!custom && app.name ? app.name : undefined}
      style={{ "--pet-icon-size": `${petSize - 4}px` } as React.CSSProperties}
    >
      <div className="pet-orb">
        {custom ? (
          <>
            <MediaImage
              key={petSrc ?? "none"}
              className="pet-fade"
              src={petSrc}
              hidden={shownWalk !== null}
              fallback={<PawPrint size={petSize - 10} />}
            />
            {walks.map(({ direction, choice }) =>
              choice ? (
                <MediaImage
                  key={`${direction}:${choice.entry}`}
                  className={choice.flip ? "pet-walk pet-flip" : "pet-walk"}
                  src={petWalkUrl(appearanceId, choice.entry)}
                  hidden={shownWalk !== direction}
                  fallback={null}
                />
              ) : null,
            )}
          </>
        ) : (
          <MediaImage
            key={app.pid ?? "none"}
            src={app.pid === null ? null : iconUrl(app.pid, petSize)}
            alt={app.name}
            fallback={<AppWindow size={petSize - 12} />}
          />
        )}
      </div>
    </div>
  );
}
