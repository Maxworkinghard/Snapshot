import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { App } from "./App";
import { PetWindow } from "./windows/PetWindow";
import { QuickMenuWindow } from "./windows/QuickMenuWindow";
import { AnnotateWindow } from "./windows/AnnotateWindow";
import { LaunchpadWindow } from "./windows/LaunchpadWindow";
import { applyTheme, listenThemeChanges, readTheme } from "./lib/theme";
import { applyMotion, listenMotionChanges, readMotionPreference, resolveMotion } from "./lib/prefs";
import "./styles/variables.css";
import "./styles.css";
import "./styles/app.css";

// 各个 WebView 独立，每个都要在首帧前套上主题与动效档位，避免浅色闪一下
applyTheme(readTheme());
applyMotion(resolveMotion(readMotionPreference()));
// 主窗口改了之后，其余窗口靠广播当场跟上，不必重开
listenThemeChanges(applyTheme);
listenMotionChanges(applyMotion);

function mount() {
  const label = getCurrentWindow().label;
  const usesTransparentSurface = label === "pet" || label === "quick-menu" || label === "launchpad";
  document.documentElement.classList.toggle("transparent-window", usesTransparentSurface);
  document.body.classList.toggle("transparent-window", usesTransparentSurface);

  const CurrentWindow =
    label === "pet" ? PetWindow
    : label === "quick-menu" ? QuickMenuWindow
    : label === "annotate" ? AnnotateWindow
    : label === "launchpad" ? LaunchpadWindow
    : App;
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <CurrentWindow />
    </StrictMode>,
  );
}

// 直接在浏览器里打开页面（npm run dev）时没有 Tauri 后端：先装一层假后端再挂载
if ("__TAURI_INTERNALS__" in window) {
  mount();
} else {
  void import("./lib/preview").then(({ installPreviewBackend }) => {
    installPreviewBackend();
    mount();
  });
}
