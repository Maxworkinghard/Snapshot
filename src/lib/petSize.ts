/** 偏好里「桌宠大小」能调的范围（百分比），和 Rust settings.rs 的 PET_SCALE_RANGE 一致 */
export const PET_SCALE = { min: 30, max: 200, step: 10, default: 100 } as const;

/**
 * 桌宠窗口的边长（逻辑像素）。100% 是以前 30% 的视觉大小，也就是屏幕短边的 6%；
 * 换屏幕跟着变。偏好里的百分比在它上面缩放，200% 到屏幕短边的 12%。
 */
export function petWindowSize(shortEdge: number, scalePercent: number): number {
  const base = shortEdge * 0.06;
  return Math.round(base * (scalePercent / 100));
}
