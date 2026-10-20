import { useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, Select } from '@fluentui/react-components';
import type { DismantleConfig, DismantleGrade } from './generated/protocol';

export const dismantleDefault: DismantleConfig = { grade: 'a_and_below', include_abandoned: false };
const grades: Record<DismantleGrade, string> = { b: 'B', a_and_below: 'A及以下', s_and_below: 'S及以下' };
export const dismantleSummary = (settings: DismantleConfig = dismantleDefault) => `${grades[settings.grade]} · ${settings.include_abandoned ? '全选已弃置' : '不全选已弃置'}`;
export function DismantleSettings({ settings = dismantleDefault, disabled, onSave }: {
  settings?: DismantleConfig; disabled: boolean; onSave: (settings: DismantleConfig) => void;
}) {
  const [draft, setDraft] = useState(settings);
  const changed = draft.grade !== settings.grade || draft.include_abandoned !== settings.include_abandoned;
  return <Card aria-label="应用组驱动盘拆解设置">
    <div className="card-title"><h2>驱动盘拆解</h2><Badge appearance="outline">按账号与应用组保存</Badge></div>
    <p>在未弃置的驱动盘中按等级选择；勾选后还会全选已弃置驱动盘。拆解会消耗所选驱动盘，每次最多处理游戏当前一批。</p>
    <Field label="未弃置驱动盘等级"><Select disabled={disabled} value={draft.grade} onChange={(_, data) => setDraft(current => ({ ...current, grade: data.value as DismantleGrade }))}>
      {Object.entries(grades).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
    </Select></Field>
    <Checkbox label="全选已弃置驱动盘" disabled={disabled} checked={draft.include_abandoned} onChange={(_, data) => setDraft(current => ({ ...current, include_abandoned: data.checked === true }))}/>
    <Button disabled={disabled || !changed} onClick={() => onSave(draft)}>保存拆解设置</Button>
  </Card>;
}
