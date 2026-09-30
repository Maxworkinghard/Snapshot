import { convertFileSrc } from "@tauri-apps/api/core";

/**
 * 后端 media:// 协议上的资源地址（见 src-tauri/src/media.rs）。
 * 图片、音频交给 WebView 按地址直接去取，不再经 IPC 传 base64 字符串。
 * convertFileSrc 负责各平台的地址形式（Windows 上是 http://media.localhost/…）。
 */
const media = (path: string) => convertFileSrc(path, "media");

export const snapshotUrl = (id: string) => media(`snapshot/${id}`);

/** 180px 高的缩略图，列表用它；老快照没有时后端现做一张 */
export const thumbUrl = (id: string) => media(`thumb/${id}`);

/** 不指定动作时由后端取该形象的默认动作 */
export const petUrl = (assetId: string, entry?: string | null) =>
  media(entry ? `pet/${assetId}/${entry}` : `pet/${assetId}`);

/** 拖动桌宠时播的走路动作；横穿画布的由后端改成原地走 */
export const petWalkUrl = (assetId: string, entry: string) => media(`pet-walk/${assetId}/${entry}`);

/** 默认动作的第一帧，静态图。形象架上用它，免得几十个 GIF 一起动 */
export const petThumbUrl = (assetId: string) => media(`pet-thumb/${assetId}`);

/** 按「显示尺寸 × 屏幕缩放」取图标：高分屏上不糊，也不多传 */
export const iconUrl = (pid: number, cssSize: number) =>
  media(`icon/${pid}/${Math.min(256, Math.ceil(cssSize * window.devicePixelRatio))}`);

/** 启动台里的应用图标。id 是后端给的应用 id，不是任意路径 */
export const launchIconUrl = (id: string, cssSize: number) =>
  media(`launch-icon/${id}/${Math.min(256, Math.ceil(cssSize * (window.devicePixelRatio || 1)))}`);

/** 后端只放行设置里选定的那一个音效文件 */
export const soundUrl = (path: string) => media(`sound/${path}`);
