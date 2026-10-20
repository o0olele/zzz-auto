import { useState } from 'react';
import { Badge, Button, Card, Field, Input } from '@fluentui/react-components';
import type { DailyConfig } from './generated/protocol';
import { dailyDefaultConfig, dailySettingsFields } from './generated/protocol';

export const dailySummary = (settings: DailyConfig = dailyDefaultConfig) => `超时 ${settings.timeout_ms} ms · 启动 / 继续延时 ${settings.start_delay_ms} ms`;
export function DailySettings({ settings = dailyDefaultConfig, disabled, onSave }: { settings?: DailyConfig; disabled: boolean; onSave: (settings: DailyConfig) => void }) {
  const [draft, setDraft] = useState({ timeout_ms: String(settings.timeout_ms), start_delay_ms: String(settings.start_delay_ms) });
  const valid = dailySettingsFields.every(field => draft[field.key].trim() !== '' && Number.isSafeInteger(Number(draft[field.key])) && Number(draft[field.key]) >= field.min && Number(draft[field.key]) <= field.max);
  const changed = dailySettingsFields.some(field => Number(draft[field.key]) !== settings[field.key]);
  return <Card aria-label="应用组日常设置">
    <div className="card-title"><h2>日常设置</h2><Badge appearance="outline">按账号与应用组保存</Badge></div>
    <p>用于活跃度奖励、丽都城募、刮刮卡、丽都周纪、卦象集录、吼吼饼铺、驱动盘拆解、兑换码和录像店。参数在启动时冻结，每个应用分别计时。预备编队检查沿用启动延时，总超时在账号预备编队设置中保存。兑换码整批共用超时；中途停止或超时保留已确认的逐码结果，下次接着处理剩余码。刮刮卡、卦象与饼铺共享游戏内每日领取次数。</p>
    <div className="input-fields">{dailySettingsFields.map(field => <Field key={field.key} label={field.label}>
      <Input type="number" min={field.min} max={field.max} value={draft[field.key]} disabled={disabled} onChange={(_, data) => setDraft(current => ({ ...current, [field.key]: data.value }))}/>
    </Field>)}</div>
    <Button disabled={disabled || !valid || !changed} onClick={() => onSave({ timeout_ms: Number(draft.timeout_ms), start_delay_ms: Number(draft.start_delay_ms) })}>保存日常设置</Button>
  </Card>;
}
