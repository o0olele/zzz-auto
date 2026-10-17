import { createContext, useContext, useEffect, useLayoutEffect, useState, type ReactNode } from 'react';

export type ThemePreference = 'system' | 'light' | 'dark';
export const interfaceScales = [90, 100, 110, 125] as const;
export type InterfaceScale = typeof interfaceScales[number];
type Preferences = { version: 1; theme: ThemePreference; scale: InterfaceScale };
type Appearance = {
  preferences: Preferences;
  dark: boolean;
  warning: string;
  update: (change: Partial<Pick<Preferences, 'theme' | 'scale'>>) => void;
  reset: () => void;
  retrySave: () => void;
};

const storageKey = 'desktop-appearance';
const defaults: Preferences = { version: 1, theme: 'system', scale: 100 };
const AppearanceContext = createContext<Appearance | null>(null);

function readPreferences(): { preferences: Preferences; warning: string } {
  try {
    const stored = localStorage.getItem(storageKey);
    if (stored !== null) {
      const value: unknown = JSON.parse(stored);
      if (typeof value !== 'object' || value === null || Array.isArray(value)) throw new Error('invalid appearance');
      const record = value as Record<string, unknown>;
      if (record.version !== 1 || (record.theme !== 'system' && record.theme !== 'light' && record.theme !== 'dark')
        || !interfaceScales.some(scale => scale === record.scale)
        || Object.keys(record).some(key => !['version', 'theme', 'scale'].includes(key))) throw new Error('invalid appearance');
      return { preferences: record as Preferences, warning: '' };
    }
    const legacy = localStorage.getItem('theme');
    return { preferences: legacy === 'dark' || legacy === 'light' ? { ...defaults, theme: legacy } : defaults, warning: '' };
  } catch {
    return { preferences: defaults, warning: '无法读取桌面外观偏好，已使用默认外观。原有存储未改写；修改外观或重试保存后会替换它。' };
  }
}

export function AppearanceProvider({ children }: { children: ReactNode }) {
  const [initial] = useState(readPreferences);
  const [preferences, setPreferences] = useState(initial.preferences);
  const [warning, setWarning] = useState(initial.warning);
  const [systemDark, setSystemDark] = useState(() => matchMedia('(prefers-color-scheme: dark)').matches);
  const dark = preferences.theme === 'system' ? systemDark : preferences.theme === 'dark';

  useEffect(() => {
    const media = matchMedia('(prefers-color-scheme: dark)');
    const changed = () => setSystemDark(media.matches);
    changed();
    media.addEventListener('change', changed);
    return () => media.removeEventListener('change', changed);
  }, []);

  useLayoutEffect(() => {
    const root = document.documentElement;
    root.dataset.theme = dark ? 'dark' : 'light';
    root.dataset.scale = String(preferences.scale);
    root.style.setProperty('zoom', String(preferences.scale / 100));
    root.style.setProperty('--ui-scale', String(preferences.scale / 100));
    const resized = () => {
      const width = window.innerWidth / (preferences.scale / 100);
      root.dataset.compact = String(width <= 1050);
      root.dataset.narrow = String(width <= 850);
      root.dataset.short = String(window.innerHeight / (preferences.scale / 100) <= 900);
    };
    resized();
    window.addEventListener('resize', resized);
    return () => window.removeEventListener('resize', resized);
  }, [dark, preferences.scale]);

  function save(next: Preferences) {
    setPreferences(next);
    try {
      localStorage.setItem(storageKey, JSON.stringify(next));
      setWarning('');
    } catch {
      setWarning('当前外观已应用，但无法保存到本地存储。重启后可能恢复原值；存储恢复后可重试保存。');
    }
  }

  return <AppearanceContext.Provider value={{ preferences, dark, warning, update: change => save({ ...preferences, ...change }), reset: () => save(defaults), retrySave: () => save(preferences) }}>{children}</AppearanceContext.Provider>;
}

export function useAppearance(): Appearance {
  const value = useContext(AppearanceContext);
  if (!value) throw new Error('缺少桌面外观上下文');
  return value;
}
