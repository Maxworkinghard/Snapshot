import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  copyText,
  getRecordingStatus,
  getShortcutConflicts,
  hideQuickMenu,
  listWindows,
  loadSettings,
  onPaletteOpened,
  onSettingsChanged,
  polishText,
  readClipboardText,
  resizeQuickMenu,
  runAction,
  showMainWindow,
  toggleRecording,
} from "../lib/backend";
import { iconUrl, petThumbUrl, petUrl } from "../lib/media";
import { formatDuration } from "../lib/format";
import { typingEntry } from "../lib/petNames";
import { motionReduced } from "../lib/motion";
import { MediaImage } from "../components/MediaImage";
import { Keys } from "../components/ui/Keys";
import type { CapturableWindow, RecordingStatus, Settings, ShortcutAction } from "../types";

type View = "menu" | "windows" | "polish";
type Status = { kind: "busy" | "ok" | "error"; text: string };
type Item = { id: string; label: ReactNode; hint?: ReactNode; keywords: string; run: () => void; muted?: boolean };

/** 成功类提示停留多久后自动收起 */
const OK_HIDE_DELAY_MS = 900;
/** 面板上方留给猫的空间（窗口本身透明） */
const CAT_ROOM = 112;
/** 面板左右和下方的透明边 */
const EDGE = 40;
/** 各视图的面板宽度：菜单装得下一行动作和快捷键就够；窗口列表多给点看标题；润色要放下底栏四个按钮 */
const PANEL_WIDTH: Record<View, number> = { menu: 320, windows: 400, polish: 520 };

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/**
 * 输入框：右键桌宠或按「呼出输入框」快捷键出来。
 * 能做的事列成几行；打字就筛选，粘贴一段文字就直接进润色。
 * 键盘就能走完：↑ ↓ 选、↵ 执行、Esc 返回或关掉；润色时 Tab 换规则、Ctrl R 再来一次、Ctrl ← 看原文。
 */
export function QuickMenuWindow() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [view, setView] = useState<View>("menu");
  // 往里走（选窗口、润色）新内容从右边进来，退回菜单从左边回来；刚打开时不额外动
  const previousView = useRef<View>("menu");
  const enterClass = useRef("");
  if (previousView.current !== view) {
    enterClass.current = view === "menu" ? "enter-back" : "enter-forward";
    previousView.current = view;
  }
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);
  const [pickerMode, setPickerMode] = useState<"capture" | "record">("capture");
  const [windows, setWindows] = useState<CapturableWindow[] | null>(null);
  const [recording, setRecording] = useState<RecordingStatus>({ active: false, target: null, startedAt: null });
  const [conflicts, setConflicts] = useState<string[]>([]);
  const [status, setStatus] = useState<Status | null>(null);
  const [opening, setOpening] = useState(0);
  const [now, setNow] = useState(() => Date.now());

  // 润色
  const [source, setSource] = useState("");
  const [result, setResult] = useState<string | null>(null);
  const [polishError, setPolishError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [peek, setPeek] = useState(false);
  const [templateId, setTemplateId] = useState<string | null>(null);
  const [startedAt, setStartedAt] = useState(0);
  const runToken = useRef(0);

  const inputRef = useRef<HTMLInputElement>(null);
  const resultRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const hideTimer = useRef<number | null>(null);

  const refresh = useCallback(() => {
    loadSettings().then(setSettings).catch(() => {});
    getRecordingStatus().then((value) => value && setRecording(value)).catch(() => {});
    getShortcutConflicts()
      .then((list) => setConflicts(Array.isArray(list) ? list : []))
      .catch(() => {});
  }, []);

  const reset = useCallback(() => {
    if (hideTimer.current !== null) window.clearTimeout(hideTimer.current);
    runToken.current += 1;
    previousView.current = "menu";
    enterClass.current = "";
    setView("menu");
    setQuery("");
    setCursor(0);
    setStatus(null);
    setWindows(null);
    setResult(null);
    setPolishError(null);
    setRunning(false);
    setPeek(false);
    setOpening((value) => value + 1);
    window.setTimeout(() => inputRef.current?.focus(), 0);
  }, []);

  useEffect(() => {
    refresh();
    const opened = onPaletteOpened(() => {
      reset();
      refresh();
    });
    const settingsListener = onSettingsChanged(setSettings);
    const focus = getCurrentWindow().onFocusChanged(({ payload }) => {
      if (!payload) void hideQuickMenu();
    });
    return () => {
      void opened.then((unlisten) => unlisten()).catch(() => {});
      void settingsListener.then((unlisten) => unlisten()).catch(() => {});
      void focus.then((unlisten) => unlisten()).catch(() => {});
      // 自动收起那次 hideQuickMenu 还挂在计时器上。不撤掉的话，窗口已经关了它才触发，
      // 对着已经卸掉的 WebView 发调用（测试里表现为卸载后的 unhandled rejection）。
      if (hideTimer.current !== null) {
        window.clearTimeout(hideTimer.current);
        hideTimer.current = null;
      }
    };
  }, [refresh, reset]);

  useEffect(() => {
    if (!recording.active && !running) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [recording.active, running]);

  const asset = settings?.petAssets.find((item) => item.id === settings.selectedAppearanceId && !item.missing) ?? null;
  const headroom = asset ? CAT_ROOM : 24;

  // 窗口大小跟着面板走：换视图时变宽变窄，进润色、展开窗口列表时变高。
  // 每次打开都会重新挂一个面板（key={opening}），要改量新的那个；旧面板移出页面时量到的是 0，不能拿来缩窗口。
  // 用 offsetWidth / offsetHeight：getBoundingClientRect 会把入场动画的 scale(0.97) 算进去，量小一圈。
  useLayoutEffect(() => {
    const panel = panelRef.current;
    if (!panel) return;
    const apply = () => {
      if (!panel.isConnected || panel.offsetHeight === 0) return;
      void resizeQuickMenu(panel.offsetWidth + EDGE * 2, panel.offsetHeight + headroom + EDGE).catch(() => {});
    };
    apply();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(apply);
    observer.observe(panel);
    return () => observer.disconnect();
    // view 变了宽度就变（PANEL_WIDTH），当场重量一次，不等 ResizeObserver
  }, [headroom, opening, view]);

  function showStatus(next: Status | null, hideAfter = false) {
    if (hideTimer.current !== null) window.clearTimeout(hideTimer.current);
    setStatus(next);
    if (hideAfter) hideTimer.current = window.setTimeout(() => void hideQuickMenu(), OK_HIDE_DELAY_MS);
  }

  const accelerator = (action: ShortcutAction) =>
    settings?.shortcuts.find((item) => item.action === action)?.accelerator ?? null;
  const shortcutHint = (action: ShortcutAction) => {
    const value = accelerator(action);
    if (!value) return null;
    if (conflicts.includes(value)) return <span className="signal small">快捷键被占用</span>;
    return <Keys value={value} quiet />;
  };

  async function openPicker(mode: "capture" | "record") {
    setPickerMode(mode);
    setView("windows");
    setQuery("");
    setCursor(0);
    setWindows(null);
    showStatus(null);
    try {
      setWindows(await listWindows());
    } catch (error) {
      setWindows([]);
      showStatus({ kind: "error", text: errorText(error) });
    }
    inputRef.current?.focus();
  }

  async function pickWindow(item: CapturableWindow) {
    if (pickerMode === "capture") {
      // 后端先把输入框藏起来再截，免得截进画面；失败会记进活动，主窗口的猫会说
      void runAction("capture", item.id).catch(() => {});
      return;
    }
    showStatus({ kind: "busy", text: `正在开始录 ${item.appName}…` });
    try {
      const next = await toggleRecording(item.id);
      setRecording(next);
      setView("menu");
      showStatus({ kind: "ok", text: next.message?.trim() || `开始录制 ${next.target ?? item.appName}` }, true);
    } catch (error) {
      showStatus({ kind: "error", text: errorText(error) });
    }
  }

  async function stopRecording() {
    showStatus({ kind: "busy", text: "正在停止并保存录像…" });
    try {
      const next = await toggleRecording();
      setRecording(next);
      showStatus({ kind: "ok", text: next.active ? "录制还在继续" : "录制已保存" }, !next.active);
    } catch (error) {
      showStatus({ kind: "error", text: errorText(error) });
    }
  }

  const run = useCallback(
    async (text: string, template: string | null) => {
      const token = ++runToken.current;
      setRunning(true);
      setPolishError(null);
      setPeek(false);
      setStartedAt(Date.now());
      try {
        const polished = await polishText(text, template);
        if (token !== runToken.current) return;
        setResult(polished);
      } catch (error) {
        if (token !== runToken.current) return;
        setPolishError(errorText(error));
      } finally {
        if (token === runToken.current) setRunning(false);
      }
    },
    [],
  );

  function enterPolish(text: string) {
    const trimmed = text.trim();
    if (!trimmed) return;
    const template = settings?.activeTemplateId ?? null;
    setSource(trimmed);
    setResult(null);
    setTemplateId(template);
    setView("polish");
    showStatus(null);
    if (!settings?.baseUrl || !settings?.model) {
      setPolishError("还没填模型接口，润色用不了。打开主窗口，在「偏好设置」里填。");
      return;
    }
    void run(trimmed, template);
  }

  async function polishClipboardText() {
    try {
      enterPolish(await readClipboardText());
    } catch (error) {
      showStatus({ kind: "error", text: errorText(error) });
    }
  }

  function cycleRule(step: number) {
    const templates = settings?.templates ?? [];
    if (templates.length < 2) return;
    const index = Math.max(0, templates.findIndex((item) => item.id === templateId));
    const next = templates[(index + step + templates.length) % templates.length];
    setTemplateId(next.id);
    if (settings?.baseUrl && settings.model) void run(source, next.id);
  }

  async function copyAndClose() {
    if (!result) return;
    try {
      await copyText(result);
      void hideQuickMenu();
    } catch (error) {
      showStatus({ kind: "error", text: errorText(error) });
    }
  }

  const items = useMemo<Item[]>(() => {
    const list: Item[] = [
      {
        id: "capture",
        label: "快照",
        hint: shortcutHint("snapshot"),
        keywords: "截图 截屏 窗口 快照 capture snapshot",
        run: () => void openPicker("capture"),
      },
      {
        id: "fullscreen",
        label: "全屏截图",
        hint: shortcutHint("fullscreen"),
        keywords: "全屏 截图 截屏 屏幕 fullscreen",
        run: () => void runAction("fullscreen").catch(() => {}),
      },
      recording.active
        ? {
            id: "record",
            label: <>停止录制 · {recording.target ?? "窗口"}</>,
            // 时长放在右边：窗口名太长时左边会被截掉，时长不能跟着没了
            hint: (
              <>
                <span className="mono signal">{formatDuration(now - (recording.startedAt ?? now))}</span>
                {shortcutHint("record")}
              </>
            ),
            keywords: "停止 录制 录屏 record stop",
            run: () => void stopRecording(),
          }
        : {
            id: "record",
            label: "录制",
            hint: shortcutHint("record"),
            keywords: "录制 录屏 视频 record",
            run: () => void openPicker("record"),
          },
      {
        id: "polish",
        label: "润色 Prompt",
        hint: shortcutHint("polish"),
        keywords: "润色 改写 prompt 剪贴板 polish",
        run: () => void polishClipboardText(),
      },
    ];
    list.push({
      id: "main",
      label: "打开主窗口",
      keywords: "主窗口 设置 历史 形象 伴侣 settings",
      muted: true,
      run: () => {
        void showMainWindow();
        void hideQuickMenu();
      },
    });
    return list;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings, recording, conflicts, now]);

  const visibleItems = useMemo<Item[]>(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return items;
    const matches = items.filter((item) => item.keywords.toLowerCase().includes(needle));
    const polishItem: Item = {
      id: "polish-query",
      label: (
        <>
          润色这段文字<span className="small quiet"> · {query.trim().length} 字</span>
        </>
      ),
      hint: <span className="mono small quiet">↵</span>,
      keywords: "",
      run: () => enterPolish(query),
    };
    // 打的是一句话而不是命令：润色排第一
    return needle.length >= 8 || matches.length === 0 ? [polishItem, ...matches] : [...matches, polishItem];
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [items, query, settings]);

  const visibleWindows = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return (windows ?? []).filter(
      (item) => !needle || item.appName.toLowerCase().includes(needle) || item.title.toLowerCase().includes(needle),
    );
  }, [windows, query]);

  // 进润色后键盘焦点落在结果区，Tab / ↵ / Esc 才接得住
  useEffect(() => {
    if (view === "polish") resultRef.current?.focus();
  }, [view]);

  const rowCount = view === "menu" ? visibleItems.length : view === "windows" ? visibleWindows.length : 0;
  useEffect(() => setCursor(0), [query, view]);

  function onKeyDown(event: React.KeyboardEvent) {
    if (view === "polish") {
      if (event.key === "Escape") {
        event.preventDefault();
        runToken.current += 1;
        setRunning(false);
        setView("menu");
        setQuery("");
        window.setTimeout(() => inputRef.current?.focus(), 0);
      } else if (event.key === "Tab") {
        event.preventDefault();
        cycleRule(event.shiftKey ? -1 : 1);
      } else if (event.key === "Enter" && !event.shiftKey) {
        event.preventDefault();
        void copyAndClose();
      } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "r") {
        event.preventDefault();
        if (!running) void run(source, templateId);
      } else if ((event.ctrlKey || event.metaKey) && event.key === "ArrowLeft") {
        event.preventDefault();
        setPeek((value) => !value);
      }
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setCursor((value) => (rowCount ? (value + 1) % rowCount : 0));
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setCursor((value) => (rowCount ? (value - 1 + rowCount) % rowCount : 0));
    } else if (event.key === "Enter") {
      event.preventDefault();
      if (view === "menu") visibleItems[cursor]?.run();
      else if (visibleWindows[cursor]) void pickWindow(visibleWindows[cursor]);
    } else if (event.key === "Escape") {
      event.preventDefault();
      if (view === "windows") {
        setView("menu");
        setQuery("");
      } else if (query) setQuery("");
      else void hideQuickMenu();
    }
  }

  const templates = settings?.templates ?? [];
  const templateName = templates.find((item) => item.id === templateId)?.name ?? "默认规则";
  const catEntry = asset ? (view === "polish" ? typingEntry(asset.animations) ?? asset.entry : asset.entry) : null;
  const catSrc = asset ? (motionReduced() ? petThumbUrl(asset.id) : petUrl(asset.id, catEntry || null)) : null;

  return (
    <div
      className="palette-root"
      style={{ paddingTop: headroom }}
      onMouseDown={(event) => {
        // 点在面板外面（透明区域）就收起
        if (event.target === event.currentTarget) void hideQuickMenu();
      }}
      onKeyDown={onKeyDown}
    >
      <div className="palette-wrap" key={opening} style={{ width: PANEL_WIDTH[view] }}>
        {catSrc && <img className="palette-cat" key={catSrc} src={catSrc} alt="" draggable={false} />}
        <div className="palette" ref={panelRef} role="dialog" aria-label="应用快照输入框">
          {view === "polish" ? (
            <div className={`palette-polish ${enterClass.current}`}>
              <div className="palette-source small">
                <span className="quiet">原文</span>
                <span className="palette-source-text">{source}</span>
                {result && <span className="mono quiet">{source.length} → {result.length} 字</span>}
              </div>
              <div className="palette-result" tabIndex={-1} ref={resultRef} aria-live="polite">
                {running ? (
                  <p className="palette-busy">
                    <span className="busy-line" aria-hidden="true" />
                    正在按「{templateName}」润色 · <span className="mono">{Math.floor((now - startedAt) / 1000)}</span> 秒
                  </p>
                ) : polishError ? (
                  <p className="signal">
                    {polishError}　{settings?.baseUrl && settings.model && (
                      <button type="button" className="link" onClick={() => void run(source, templateId)}>重试</button>
                    )}
                  </p>
                ) : (
                  <p className="palette-result-text">{peek ? source : result}</p>
                )}
              </div>
              <div className="palette-foot small">
                <button type="button" className="text-btn palette-rule" onClick={() => cycleRule(1)} disabled={templates.length < 2}>
                  <span className="strong">{templateName}</span>
                  {templates.length > 1 && <span className="mono quiet">Tab</span>}
                </button>
                <span className="spacer" />
                <button type="button" className="text-btn ink-2" onClick={() => setPeek((value) => !value)} disabled={!result}>
                  {peek ? "看结果" : "看原文"}<span className="mono quiet">Ctrl ←</span>
                </button>
                <button type="button" className="text-btn ink-2" onClick={() => void run(source, templateId)} disabled={running}>
                  再来一次<span className="mono quiet">Ctrl R</span>
                </button>
                <button type="button" className="btn btn-primary" onClick={() => void copyAndClose()} disabled={!result || running}>
                  复制并关闭<span className="mono">↵</span>
                </button>
              </div>
            </div>
          ) : (
            <>
              <div className="palette-input-row">
                {view === "windows" && (
                  <button type="button" className="palette-back" aria-label="返回" onClick={() => { setView("menu"); setQuery(""); }}>
                    <svg width="7" height="12" viewBox="0 0 7 12" aria-hidden="true"><path d="M6 1L1 6l5 5" fill="none" stroke="currentColor" strokeWidth="1.4" /></svg>
                  </button>
                )}
                <input
                  ref={inputRef}
                  autoFocus
                  className="palette-input"
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  onPaste={(event) => {
                    if (view !== "menu") return;
                    const text = event.clipboardData.getData("text");
                    // 粘进来的是一段话：不用再按回车，直接去润色
                    if (text.includes("\n") || text.trim().length > 40) {
                      event.preventDefault();
                      enterPolish(text);
                    }
                  }}
                  placeholder={
                    view === "windows" ? (pickerMode === "capture" ? "截哪个窗口" : "录哪个窗口") : "要做什么？"
                  }
                  aria-label={view === "windows" ? "筛选窗口" : "命令，或要润色的文字"}
                  spellCheck={false}
                />
                <span className="mono small quiet">Esc</span>
              </div>

              {view === "menu" ? (
                <div className={`palette-list ${enterClass.current}`} role="listbox" aria-label="动作">
                  {visibleItems.map((item, index) => (
                    <button
                      key={item.id}
                      type="button"
                      role="option"
                      aria-selected={index === cursor}
                      className={`palette-item ${index === cursor ? "is-on" : ""} ${item.muted ? "is-muted" : ""} ${item.id === "main" && index > 0 ? "is-apart" : ""}`}
                      onMouseMove={() => setCursor(index)}
                      onClick={item.run}
                    >
                      <span className="palette-label">{item.label}</span>
                      {item.hint && <span className="palette-hint">{item.hint}</span>}
                    </button>
                  ))}
                </div>
              ) : (
                <div className={`palette-list palette-windows ${enterClass.current}`} role="listbox" aria-label={pickerMode === "capture" ? "选择要截取的窗口" : "选择要录制的窗口"}>
                  {windows === null && <p className="palette-empty small quiet">正在读取窗口…</p>}
                  {windows !== null && visibleWindows.length === 0 && (
                    <p className="palette-empty small quiet">{query ? "没有匹配的窗口" : "没有找到可用的窗口"}</p>
                  )}
                  {visibleWindows.map((item, index) => (
                    <button
                      key={item.id}
                      type="button"
                      role="option"
                      aria-selected={index === cursor}
                      className={`palette-item ${index === cursor ? "is-on" : ""}`}
                      onMouseMove={() => setCursor(index)}
                      onClick={() => void pickWindow(item)}
                    >
                      <span className="palette-window">
                        <span className="palette-icon">
                          <MediaImage src={iconUrl(item.pid, 20)} fallback={<span className="palette-icon-blank" />} />
                        </span>
                        <span className="palette-window-name">{item.appName}</span>
                        <span className="palette-window-title small quiet">{item.title}</span>
                      </span>
                    </button>
                  ))}
                </div>
              )}
            </>
          )}
          {status && (
            <div className={`palette-status small is-${status.kind}`} role="status">
              {status.kind === "busy" && <span className="busy-dot" aria-hidden="true" />}
              {status.text}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
