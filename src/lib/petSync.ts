import { emit, listen } from "@tauri-apps/api/event";

/**
 * 桌宠正在播哪个动作。桌宠窗口是动作的「源头」（平时待机，点一下换下一个，3 秒后回到待机），
 * 主窗口侧栏的猫跟着它播，两边看到的永远是同一个动作。
 */
export interface PetAnimation {
  assetId: string;
  entry: string | null;
}

const CHANGED = "pet-animation";
const ASK = "pet-animation-ask";
const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** 桌宠换动作时广播 */
export function broadcastPetAnimation(value: PetAnimation) {
  if (!inTauri) return;
  void emit(CHANGED, value);
}

/** 主窗口刚打开时问一声：桌宠现在播的是哪个 */
export function askPetAnimation() {
  if (!inTauri) return;
  void emit(ASK);
}

export function onPetAnimation(callback: (value: PetAnimation) => void): () => void {
  if (!inTauri) return () => {};
  const pending = listen<PetAnimation>(CHANGED, ({ payload }) => callback(payload));
  return () => void pending.then((unlisten) => unlisten());
}

export function onPetAnimationAsked(callback: () => void): () => void {
  if (!inTauri) return () => {};
  const pending = listen(ASK, () => callback());
  return () => void pending.then((unlisten) => unlisten());
}
