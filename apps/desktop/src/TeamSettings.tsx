import { useState } from 'react';
import { Badge, Button, Card, Field, Input, Select } from '@fluentui/react-components';
import { teamDefaultConfig, type PredefinedTeam, type TeamConfig, type TeamCheckProgress } from './generated/protocol';
import agents from '../../../resources/source/random-play/agents.json';

const agentName = (id: string) => agents.find(agent => agent.id === id)?.name ?? (id === 'unknown' ? '未知 / 空位' : id);
export function TeamSummary({ settings = teamDefaultConfig }: { settings?: TeamConfig }) {
  return <details aria-label="预备编队配置明细"><summary>预备编队：{settings.teams.length} 个槽位 · 检查超时 {settings.check_timeout_ms ?? 300000} ms</summary>
    <ol>{settings.teams.map((team, index) => <li key={index}>{team.name} · {team.agent_ids.map(agentName).join(' / ')} · {team.auto_battle}</li>)}</ol>
  </details>;
}

export function TeamSettings({ settings = teamDefaultConfig, disabled, onSave }: {
  settings?: TeamConfig; disabled: boolean; onSave: (settings: TeamConfig) => void;
}) {
  const [draft, setDraft] = useState(settings);
  const [selected, setSelected] = useState(0);
  const team = draft.teams[selected];
  const update = (change: Partial<PredefinedTeam>) => setDraft(current => ({
    ...current,
    teams: current.teams.map((entry, index) => index === selected ? { ...entry, ...change } : entry),
  }));
  const timeout = draft.check_timeout_ms ?? 300000;
  const valid = Number.isSafeInteger(timeout) && timeout >= 1000 && timeout <= 900000 && draft.teams.length === 20
    && draft.teams.every(entry => entry.name.length > 0 && [...entry.name].length <= 80
      && entry.name.trim() === entry.name && !/[\u0000-\u001f\u007f-\u009f]/u.test(entry.name)
      && entry.auto_battle.length > 0 && new TextEncoder().encode(entry.auto_battle).length <= 240
      && entry.auto_battle.trim() === entry.auto_battle && !['.', '..'].includes(entry.auto_battle)
      && !/[/\\:\u0000-\u001f\u007f-\u009f]/u.test(entry.auto_battle)
      && entry.agent_ids.every(id => id === 'unknown' || agents.some(agent => agent.id === id))
      && new Set(entry.agent_ids.filter(id => id !== 'unknown')).size === entry.agent_ids.filter(id => id !== 'unknown').length);
  const changed = JSON.stringify(draft) !== JSON.stringify(settings);
  return <Card aria-label="账号预备编队设置">
    <div className="card-title"><h2>预备编队</h2><Badge appearance="outline">按账号保存</Badge></div>
    <p>20 个槽位与游戏中的编队顺序对应。每个槽位保存名称、战斗脚本和三名成员，未知成员可暂留空位。</p>
    <Field label="编队检查总超时（ms）"><Input type="number" min={1000} max={900000} value={String(timeout)} disabled={disabled}
      onChange={(_, data) => setDraft(current => ({ ...current, check_timeout_ms: data.value === '' ? 0 : Number(data.value) }))}/></Field>
    <Field label="编辑编队槽位"><Select value={String(selected)} disabled={disabled} onChange={(_, data) => setSelected(Number(data.value))}>
      {draft.teams.map((entry, index) => <option key={index} value={index}>{index + 1} · {entry.name}</option>)}
    </Select></Field>
    <Field label="编队名称"><Input value={team.name} disabled={disabled} onChange={(_, data) => update({ name: data.value })}/></Field>
    <Field label="编队战斗脚本"><Input value={team.auto_battle} disabled={disabled} onChange={(_, data) => update({ auto_battle: data.value })}/></Field>
    {team.agent_ids.map((id, slot) => <Field key={slot} label={`编队成员 ${slot + 1}`}>
      <Select value={id} disabled={disabled} onChange={(_, data) => {
        const ids = [...team.agent_ids] as [string, string, string]; ids[slot] = data.value; update({ agent_ids: ids });
      }}><option value="unknown">未知 / 空位</option>{agents.map(agent => <option key={agent.id} value={agent.id}>{agent.name}</option>)}</Select>
    </Field>)}
    {!valid && <p role="alert">检查超时须为 1000 至 900000 ms 的整数；请使用有效队名和脚本名称，并避免同队重复代理人。</p>}
    <Button disabled={disabled || !valid || !changed} onClick={() => onSave(draft)}>保存预备编队</Button>
    <TeamSummary settings={draft}/>
  </Card>;
}

export function TeamCheckStatus({ progress }: { progress: TeamCheckProgress }) {
  const names = (indices: number[]) => indices.map(index => progress.config.teams[index]?.name ?? `槽位 ${index + 1}`).join('、');
  return <section aria-label="预备编队检查进度">
    <p>编队扫描：{progress.pages_read} / 5 页 · 已确认 {progress.members.length} 队 · {progress.saved_revision == null ? '尚未写回配置' : `已保存至修订 ${progress.saved_revision}`}</p>
    {progress.pages_read === 0 && progress.opened && <p>正在将列表滚回顶部：{progress.top_scrolls} / 4</p>}
    {progress.conflicted.length > 0 && <p>结果冲突，保留原成员：{names(progress.conflicted)}</p>}
    {progress.unreadable.length > 0 && <p>未能识别，保留原成员：{names(progress.unreadable)}</p>}
    <ul>{progress.members.map(member => <li key={member.index}>{member.index + 1} · {member.observed_name ?? member.name}：{member.agent_ids.map(agentName).join(' / ')}</li>)}</ul>
  </section>;
}
