/**
 * 启动台横向翻页。
 * 正的 drag、正的速度都表示手指向右，内容跟着右移，露出上一页。
 * 滚轮向下或向右时 delta 为正，内容改去下一页（drag 减小）。一颗滚轮格翻一页。
 * 松手落在哪一页只由位移和速度决定；弹簧从当前位移起步，不提前跳到前瞻位置。
 */

export const COMMIT_FRACTION = 0.22;
export const FLICK_VELOCITY = 0.55;
export const LOOKAHEAD_MS = 120;
export const VELOCITY_WINDOW_MS = 80;
/** 停手这么久，这一串滚轮就算结束。 */
export const WHEEL_QUIET_MS = 400;
/** 这个间隔里的一串事件算同一格。Windows 经常把一格拆成好几下上报。 */
export const WHEEL_CLUSTER_MS = 80;
export const SPRING_OMEGA = 0.018;
export const SPRING_ZETA = 0.72;
export const SPRING_DT_CAP = 32;
export const SPRING_SUBSTEP = 4;
export const SPRING_SETTLE_X = 0.6;
export const SPRING_SETTLE_V = 0.02;
export const SPRING_VELOCITY_CAP = 1.6;

export type Spring = { x: number; v: number; target: number };
export type Sample = { t: number; x: number };

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

/** 拉过两端时的橡皮筋。越拉越沉，最多接近页宽的 0.42。 */
export function rubberBand(offset: number, pageWidth: number) {
  const distance = Math.max(0, offset);
  const width = Math.max(1, pageWidth);
  const limit = width * 0.42;
  const stiffness = 0.55;
  return (distance * limit * stiffness) / (limit + stiffness * distance);
}

/** 第 page 页加上拖动后的轨道位移。范围内 1:1，出了两端才橡皮筋。 */
export function trackX(page: number, pageCount: number, drag: number, pageWidth: number) {
  const width = Math.max(1, pageWidth);
  const last = Math.max(0, pageCount - 1);
  const index = clamp(page, 0, last);
  const raw = -index * width + drag;
  const min = -last * width;
  if (raw > 0) return rubberBand(raw, width);
  if (raw < min) return min - rubberBand(min - raw, width);
  return raw;
}

function pageSteps(drag: number, width: number) {
  const moved = -drag;
  if (!Number.isFinite(moved) || moved === 0) return 0;
  const direction = moved > 0 ? 1 : -1;
  const distance = Math.abs(moved);
  const whole = Math.floor(distance / width);
  const fraction = distance - whole * width;
  const steps = fraction >= width * COMMIT_FRACTION ? whole + 1 : whole;
  return direction * steps;
}

/**
 * 松手后落到哪一页。拖过页宽的 22%，或速度达到甩动阈值，就换页。
 * 两者方向相反时听速度。手指已经拖过整页时按越过的页数落地，不在同方向上再加一页。
 * 没到甩动阈值时，用一小段前瞻补「差一点越过 22%」；前瞻不改变弹簧的起点。
 */
export function targetPage(page: number, pageCount: number, drag: number, velocity: number, pageWidth: number) {
  const width = Math.max(1, pageWidth);
  const max = Math.max(0, pageCount - 1);
  const origin = clamp(Math.round(page) || 0, 0, max);
  if (max === 0) return 0;
  const flicked = Math.abs(velocity) >= FLICK_VELOCITY;
  const looked = flicked || !Number.isFinite(velocity) ? drag : drag + velocity * LOOKAHEAD_MS;
  const byDistance = pageSteps(looked, width);
  const flickStep = !flicked ? 0 : velocity < 0 ? 1 : -1;
  let delta = byDistance;
  if (flickStep !== 0) {
    const distDir = Math.sign(byDistance);
    if (byDistance === 0 || distDir !== flickStep) delta = flickStep;
  }
  return clamp(origin + delta, 0, max);
}

export function clampSpringVelocity(velocity: number) {
  if (!Number.isFinite(velocity)) return 0;
  return clamp(velocity, -SPRING_VELOCITY_CAP, SPRING_VELOCITY_CAP);
}

/**
 * 半隐式欧拉。帧间隔封顶 32ms，内部按 4ms 分步，
 * 否则大步长会把这组欠阻尼参数的过冲积掉。
 */
export function stepSpring(spring: Spring, dtMs: number): Spring {
  let x = spring.x;
  let v = spring.v;
  const target = spring.target;
  let remaining = Math.min(SPRING_DT_CAP, Math.max(0, dtMs));
  if (remaining === 0 || !Number.isFinite(x) || !Number.isFinite(v)) return spring;
  while (remaining > 0) {
    const dt = Math.min(SPRING_SUBSTEP, remaining);
    remaining -= dt;
    const accel = -SPRING_OMEGA * SPRING_OMEGA * (x - target) - 2 * SPRING_ZETA * SPRING_OMEGA * v;
    v += accel * dt;
    x += v * dt;
  }
  return { x, v, target };
}

export function springSettled(spring: Spring) {
  return Math.abs(spring.x - spring.target) < SPRING_SETTLE_X && Math.abs(spring.v) < SPRING_SETTLE_V;
}

/** 最近一段采样的速度（px/ms）。窗口只用于判断，不参与弹簧起点。 */
export function gestureVelocity(samples: Sample[], now: number) {
  const recent = samples.filter((sample) => sample.t <= now && now - sample.t <= VELOCITY_WINDOW_MS);
  if (recent.length < 2) return 0;
  const first = recent[0];
  const last = recent[recent.length - 1];
  const dt = last.t - first.t;
  if (dt <= 0) return 0;
  return (last.x - first.x) / dt;
}

export type WheelDetents = { count: number; at: number; direction: number; armed: boolean };

export function emptyWheelDetents(): WheelDetents {
  return { count: 0, at: 0, direction: 0, armed: false };
}

/**
 * 记一格滚轮。同一格里连续上报、或一次特别大的 delta，都只算一格。
 * 隔开再滚，才是下一格。一格就是一页。
 */
export function noteWheelDetent(state: WheelDetents, delta: number, now: number): WheelDetents {
  const direction = Math.sign(delta);
  if (!direction || !Number.isFinite(now)) return state;
  const same = state.armed && state.direction === direction && now - state.at <= WHEEL_CLUSTER_MS;
  if (same) return { ...state, at: now };
  return { count: state.count + direction, at: now, direction, armed: true };
}

/** 一格翻一整页。 */
export function wheelDetentTravel(detents: number, pageWidth: number) {
  const width = Math.max(1, pageWidth);
  if (!Number.isFinite(detents) || detents === 0) return 0;
  return Math.sign(detents) * Math.abs(detents) * width;
}

/** 滚轮翻页的时长。先快后停，不越过目标。 */
export const WHEEL_EASE_MS = 320;

/** 滚轮滑到目标页。单调靠近，不冲过终点，所以没有切换前的冲出和落地后的回弹。 */
export function wheelEase(from: number, to: number, elapsed: number, duration = WHEEL_EASE_MS) {
  if (!Number.isFinite(from) || !Number.isFinite(to)) return to;
  const t = clamp(elapsed / Math.max(1, duration), 0, 1);
  const eased = 1 - (1 - t) ** 3;
  return from + (to - from) * eased;
}
