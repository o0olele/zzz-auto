import { useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, Input, Select, Textarea } from '@fluentui/react-components';
import { lostVoidDefaults, lostVoidRegions, lostVoidStrategies, type LostVoidConfig, type LostVoidChallenge } from './generated/protocol';

const missions = { purge: '战线肃清', task_force: '特遣调查', matrix_action: '矩阵行动' };
const tasks = { bounty_commission: '完成悬赏委托', eval_point: '刷满业绩点', period_reward: '刷满周期奖励', weekly_plan_times: '完成周计划次数' };
const lines = (text: string) => text.split(/\r?\n/u).map(s => s.trim()).filter(Boolean);
const nameValid = (text: string) => text.length > 0 && text.trim() === text && new TextEncoder().encode(text).length <= 240 && !['.', '..'].includes(text) && !/[/\\:\u0000-\u001f\u007f-\u009f]/u.test(text);

export function LostVoidSummary({ settings = lostVoidDefaults }: { settings?: LostVoidConfig }) {
  const c = settings.challenge;
  return <section aria-label="迷失之地配置摘要">
    <p>迷失之地：{missions[settings.mission_name]} · {tasks[settings.extra_task]} · 每日上限 {settings.daily_plan_times} 次 · 每周计划 {settings.weekly_plan_times} 次 · 方案 {settings.challenge_config}</p>
    <p>战斗脚本 {c.auto_battle} · {c.predefined_team_idx < 0 ? '当前队伍' : `预备队 ${c.predefined_team_idx + 1}`} · UP 配队{c.choose_team_by_priority ? '开启' : '关闭'} · 自选代理人{c.manually_choose_agent ? `：${c.team_info.join(' / ')}` : '关闭'} · {c.investigation_strategy} · 周期增益第 {c.period_buff_no} 个 · 追新{c.chase_new_mode ? '开启' : '关闭'}</p>
    <p>NEW 优先{c.artifact_priority_new ? '开启' : '关闭'} · 第一优先级：{c.artifact_priority.join(' → ') || '空'} · 第二优先级：{c.artifact_priority_2.join(' → ') || '空'} · 区域：{c.region_type_priority.join(' → ') || '按默认顺序'}</p>
    <p>硬币购买{c.store_gold ? '开启' : '关闭'} · 血量购买{c.store_blood ? `开启，最低 ${c.store_blood_min}%` : '关闭'} · 优先级外购买阈值 {c.buy_only_priority_2 === 999 ? '一直刷新' : c.buy_only_priority_2}。完整游戏流程仍在开发。</p>
  </section>;
}

export function LostVoidSettings({ settings = lostVoidDefaults, disabled, onSave }: {
  settings?: LostVoidConfig; disabled: boolean; onSave: (settings: LostVoidConfig) => void;
}) {
  const [draft, setDraft] = useState(settings);
  const [rules, setRules] = useState({ artifact_priority: settings.challenge.artifact_priority.join('\n'), artifact_priority_2: settings.challenge.artifact_priority_2.join('\n'), region_type_priority: settings.challenge.region_type_priority.join('\n') });
  const c = draft.challenge;
  const challenge = { ...c, artifact_priority: lines(rules.artifact_priority), artifact_priority_2: lines(rules.artifact_priority_2), region_type_priority: lines(rules.region_type_priority) };
  const final = { ...draft, challenge };
  const patch = (value: Partial<LostVoidChallenge>) => setDraft(current => ({ ...current, challenge: { ...current.challenge, ...value } }));
  const integer = (value: number, min: number, max: number) => Number.isInteger(value) && value >= min && value <= max;
  const valid = integer(draft.daily_plan_times, 0, 999) && integer(draft.weekly_plan_times, 0, 999)
    && nameValid(draft.challenge_config) && nameValid(c.auto_battle) && c.team_info.length === 3 && c.team_info.every(nameValid)
    && integer(c.predefined_team_idx, -1, 19) && integer(c.store_blood_min, 0, 100)
    && Object.values(rules).every(text => lines(text).length <= 128 && lines(text).every(rule => new TextEncoder().encode(rule).length <= 240 && !/[\u0000-\u001f\u007f-\u009f]/u.test(rule)))
    && challenge.region_type_priority.every(region => (lostVoidRegions as readonly string[]).includes(region)) && new Set(challenge.region_type_priority).size === challenge.region_type_priority.length;
  return <Card aria-label="应用组迷失之地设置">
    <div className="card-title"><h2>迷失之地</h2><Badge appearance="outline">配置与决策开发中</Badge></div>
    <p>设置按当前账号和应用组保存。矩阵行动仅在对应周期可用；完整入口、跑图与战斗仍在开发。</p>
    <div className="form-grid">
      <Field label="迷失之地副本"><Select disabled={disabled} value={draft.mission_name} onChange={(_, d) => setDraft(v => ({ ...v, mission_name: d.value as LostVoidConfig['mission_name'] }))}>{Object.entries(missions).map(([value, label]) => <option key={value} value={value}>{label}</option>)}</Select></Field>
      <Field label="迷失之地周目标"><Select disabled={disabled} value={draft.extra_task} onChange={(_, d) => setDraft(v => ({ ...v, extra_task: d.value as LostVoidConfig['extra_task'] }))}>{Object.entries(tasks).map(([value, label]) => <option key={value} value={value}>{label}</option>)}</Select></Field>
      <Field label="迷失之地每日上限" hint="0 表示当天不运行；达到周目标后也停止。"><Input type="number" min={0} max={999} disabled={disabled} value={String(draft.daily_plan_times)} onChange={(_, d) => setDraft(v => ({ ...v, daily_plan_times: Number(d.value) }))}/></Field>
      <Field label="迷失之地每周计划次数" hint="周目标选择“完成周计划次数”时生效。"><Input type="number" min={0} max={999} disabled={disabled} value={String(draft.weekly_plan_times)} onChange={(_, d) => setDraft(v => ({ ...v, weekly_plan_times: Number(d.value) }))}/></Field>
    </div>
    <details><summary>迷失之地挑战方案</summary>
      <p>方案完整保存在本组；编辑名称不会切换或读取外部文件。</p>
      <Field label="迷失之地方案名称"><Input disabled={disabled} value={draft.challenge_config} onChange={(_, d) => setDraft(v => ({ ...v, challenge_config: d.value }))}/></Field>
      <Field label="迷失之地战斗脚本"><Input disabled={disabled} value={c.auto_battle} onChange={(_, d) => patch({ auto_battle: d.value })}/></Field>
      <Field label="迷失之地预备编队"><Select disabled={disabled} value={String(c.predefined_team_idx)} onChange={(_, d) => patch({ predefined_team_idx: Number(d.value) })}><option value="-1">当前队伍</option>{Array.from({ length: 20 }, (_, i) => <option key={i} value={i}>预备队 {i + 1}</option>)}</Select></Field>
      <Checkbox label="迷失之地优先选择 UP 配队" disabled={disabled} checked={c.choose_team_by_priority} onChange={(_, d) => patch({ choose_team_by_priority: d.checked === true })}/>
      <Checkbox label="迷失之地自选代理人" disabled={disabled} checked={c.manually_choose_agent} onChange={(_, d) => patch({ manually_choose_agent: d.checked === true })}/>
      {c.team_info.map((agent, i) => <Field key={i} label={`迷失之地代理人 ${i + 1}`} hint="保存代理人标识；unknown 表示未指定。"><Input disabled={disabled || !c.manually_choose_agent} value={agent} onChange={(_, d) => patch({ team_info: c.team_info.map((v, index) => index === i ? d.value : v) })}/></Field>)}
      <Field label="迷失之地调查战略"><Select disabled={disabled} value={c.investigation_strategy} onChange={(_, d) => patch({ investigation_strategy: d.value })}>{lostVoidStrategies.map(value => <option key={value} value={value}>{value}</option>)}</Select></Field>
      <Field label="迷失之地周期增益"><Select disabled={disabled} value={String(c.period_buff_no)} onChange={(_, d) => patch({ period_buff_no: Number(d.value) })}>{[1, 2, 3].map(i => <option key={i} value={i}>第 {i} 个</option>)}</Select></Field>
      <Checkbox label="迷失之地优先未满级战略" disabled={disabled} checked={c.chase_new_mode} onChange={(_, d) => patch({ chase_new_mode: d.checked === true })}/>
      <Checkbox label="迷失之地优先 NEW 藏品" disabled={disabled} checked={c.artifact_priority_new} onChange={(_, d) => patch({ artifact_priority_new: d.checked === true })}/>
      {(['artifact_priority', 'artifact_priority_2', 'region_type_priority'] as const).map((key, i) => <Field key={key} label={['迷失之地藏品第一优先级', '迷失之地藏品第二优先级', '迷失之地区域优先级'][i]} hint={i === 2 ? `每行一项，可用：${lostVoidRegions.join('、')}` : '每行一条：分类、名称、分类加名称或等级；例如：通用 S。'}><Textarea disabled={disabled} rows={4} value={rules[key]} onChange={(_, d) => setRules(v => ({ ...v, [key]: d.value }))}/></Field>)}
      <Field label="迷失之地优先级外购买阈值" hint="沿用旧配置：到第 x 轮才允许选择优先级以外的藏品；999 为一直刷新。"><Select disabled={disabled} value={String(c.buy_only_priority_2)} onChange={(_, d) => patch({ buy_only_priority_2: Number(d.value) })}>{[0, 1, 2, 3, 4, 999].map(i => <option key={i} value={i}>{i === 999 ? '一直刷新' : String(i)}</option>)}</Select></Field>
      <Checkbox label="迷失之地允许硬币购买" disabled={disabled} checked={c.store_gold} onChange={(_, d) => patch({ store_gold: d.checked === true })}/>
      <Checkbox label="迷失之地允许血量购买" disabled={disabled} checked={c.store_blood} onChange={(_, d) => patch({ store_blood: d.checked === true })}/>
      <Field label="迷失之地购买最低血量（%）"><Input type="number" min={0} max={100} disabled={disabled} value={String(c.store_blood_min)} onChange={(_, d) => patch({ store_blood_min: Number(d.value) })}/></Field>
    </details>
    {!valid && <p role="alert">请核对次数、名称、代理人和优先级；区域须为列表中的名称且不能重复。</p>}
    <Button disabled={disabled || !valid || JSON.stringify(final) === JSON.stringify(settings)} onClick={() => onSave(final)}>保存迷失之地设置</Button>
  </Card>;
}
