import { useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, Input, Select } from '@fluentui/react-components';
import { teamDefaultConfig, type IntelBoardConfig, type TeamConfig } from './generated/protocol';

export const intelBoardDefault: IntelBoardConfig = { predefined_team_idx: -1, auto_battle_config: '全配队通用', exp_grind_mode: false };
export function intelBoardSummary(settings: IntelBoardConfig = intelBoardDefault) {
  return `${settings.exp_grind_mode ? '每周经验 5000' : '每周贡献 1000'} · ${settings.predefined_team_idx < 0 ? `当前队伍 / ${settings.auto_battle_config}` : `预备编队 ${settings.predefined_team_idx + 1} / 使用该队脚本`}`;
}
export function IntelBoardSettings({ settings = intelBoardDefault, teams = teamDefaultConfig, disabled, onSave }: {
  settings?: IntelBoardConfig; teams?: TeamConfig; disabled: boolean; onSave: (settings: IntelBoardConfig) => void;
}) {
  const [draft, setDraft] = useState(settings);
  const name = draft.auto_battle_config;
  const valid = name.length > 0 && name.trim() === name && new TextEncoder().encode(name).length <= 240
    && !['.', '..'].includes(name) && !/[/\\:\u0000-\u001f\u007f-\u009f]/u.test(name);
  const changed = JSON.stringify(draft) !== JSON.stringify(settings);
  return <Card aria-label="应用组情报板设置">
    <div className="card-title"><h2>情报板</h2><Badge appearance="outline">按账号与应用组保存</Badge></div>
    <p>按区服的游戏周保存进度。普通模式以贡献达到 1000 为目标；经验模式按恶名狩猎每次 500、专业挑战室每次 250 累计到 5000。完整执行入口仍在开发。</p>
    <Checkbox label="情报板经验模式" checked={draft.exp_grind_mode} disabled={disabled}
      onChange={(_, data) => setDraft(current => ({ ...current, exp_grind_mode: data.checked === true }))}/>
    <Field label="情报板预备编队"><Select value={String(draft.predefined_team_idx)} disabled={disabled}
      onChange={(_, data) => setDraft(current => ({ ...current, predefined_team_idx: Number(data.value) }))}>
      <option value="-1">不切换，使用当前队伍</option>
      {teams.teams.map((team, index) => <option key={index} value={index}>{index + 1} · {team.name}</option>)}
    </Select></Field>
    <Field label="情报板战斗脚本" hint={draft.predefined_team_idx < 0 ? '不切换编队时采用此脚本。' : `将采用所选队伍的脚本：${teams.teams[draft.predefined_team_idx]?.auto_battle ?? '未知'}；下方名称保留供不切换编队时使用。`}>
      <Input value={name} disabled={disabled} onChange={(_, data) => setDraft(current => ({ ...current, auto_battle_config: data.value }))}/>
    </Field>
    {!valid && <p role="alert">脚本名称须非空且不含路径、控制字符或首尾空白，UTF-8 长度不超过 240 字节。</p>}
    <Button disabled={disabled || !valid || !changed} onClick={() => onSave(draft)}>保存情报板设置</Button>
  </Card>;
}
