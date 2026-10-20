import { combatDefaultConfig, type CombatConfig } from './generated/protocol';

export function combatSummary(value?: CombatConfig): string {
  const settings = value ?? combatDefaultConfig as CombatConfig;
  return `${settings.dodge_only ? '闪避助手' : '自动战斗'} · ${settings.dodge_only ? settings.dodge_script : settings.script} · ${settings.mode} · ${settings.directml_device === null ? 'CPU' : `DirectML ${settings.directml_device}`} · 声音${settings.audio ? `开启（阈值 ${settings.audio_threshold_milli / 1000}）` : '关闭'} · 截图 ${settings.frame_interval_ms} ms · 自定义映射 ${Object.keys(settings.bindings).length} 项`;
}
