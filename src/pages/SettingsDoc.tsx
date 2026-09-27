import { ShortcutsPanel, StoragePanel } from "./ShortcutsPage";
import { BehaviorPanel, MachinePanel, PetPanel } from "./PreferencesPage";
import { ThemesPanel } from "./ThemesPage";
import { RulesPanel } from "./RulesPanel";

/** 「时间线」布局里，所有设一次就不再动的东西收进这一页。宽窗口分两栏，外观横跨在最下面 */
export function SettingsDoc() {
  return (
    <div className="settings-doc">
      <div className="settings-col">
        <ShortcutsPanel />
        <StoragePanel />
      </div>
      <div className="settings-col">
        <BehaviorPanel />
        <PetPanel />
        <MachinePanel />
        <RulesPanel />
      </div>
      <div className="settings-wide">
        <ThemesPanel />
      </div>
    </div>
  );
}
