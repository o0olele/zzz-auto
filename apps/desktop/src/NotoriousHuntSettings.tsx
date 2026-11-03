import { useEffect, useRef, useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, Input, Select } from '@fluentui/react-components';
import { chargePlanCatalog, chargePlanLevels, notoriousHuntDefaults, notoriousHuntItem, teamDefaultConfig, type ChargePlanItem, type NotoriousHuntConfig, type NotoriousHuntGroupPreview, type TeamConfig } from './generated/protocol';
import { errorMessage, request } from './bridge';

const days = ['周一', '周二', '周三', '周四', '周五', '周六', '周日'];
const missions = chargePlanCatalog.find(c => c.category_name === '恶名狩猎')!.mission_type_list;
const decisions = { no_plans: '没有有效计划', before_start_day: '未到本周开始日', unknown_rewards: '剩余周奖励次数未知', exhausted: '本周奖励次数已用完', completed: '本轮计划已完成', ready: '可按此顺序核对挑战', needs_agent_target: '需先读取代理人培养目标，后续顺序待定' };
export function NotoriousHuntSummary({ settings = notoriousHuntDefaults }: { settings?: NotoriousHuntConfig }) {
  return <section aria-label="恶名狩猎设置摘要"><p>恶名狩猎：{settings.plan_list.length} 项 · {days[settings.weekly_challenge_start_weekday - 1]}开始 · 循环{settings.loop ? '开' : '关'}。</p>
    <ol>{settings.plan_list.map(p => <li key={p.plan_id}>{p.mission_type_name} · {p.level} · 鸣徽 {p.notorious_hunt_buff_num} · 基线 {p.run_times}/{p.plan_times} · {p.predefined_team_idx < 0 ? `当前队伍 / ${p.auto_battle_config}` : `预备编队 ${p.predefined_team_idx + 1} / 使用编队脚本`}</li>)}</ol>
  </section>;
}
type Draft = Omit<ChargePlanItem, 'run_times' | 'plan_times'> & { run_times: string; plan_times: string };
const draft = (p: ChargePlanItem): Draft => ({ ...p, run_times: String(p.run_times), plan_times: String(p.plan_times) });
const item = (p: Draft): ChargePlanItem => ({ ...p, run_times: Number(p.run_times), plan_times: Number(p.plan_times) });
const integer = (s: string, max: number) => /^(0|[1-9]\d*)$/.test(s) && Number(s) <= max;
const valid = (p: Draft) => integer(p.run_times, 1000000) && integer(p.plan_times, 1000)
  && p.auto_battle_config.length > 0 && p.auto_battle_config.trim() === p.auto_battle_config
  && new TextEncoder().encode(p.auto_battle_config).length <= 240 && !['.', '..'].includes(p.auto_battle_config)
  && !/[/\\:\u0000-\u001f\u007f-\u009f]/u.test(p.auto_battle_config);

export function NotoriousHuntSettings({ settings = notoriousHuntDefaults, teams = teamDefaultConfig, instanceId, groupId, revision, disabled, onSave }: {
  settings?: NotoriousHuntConfig; teams?: TeamConfig; instanceId: number; groupId: string; revision: number; disabled: boolean; onSave: (settings: NotoriousHuntConfig) => void;
}) {
  const [options, setOptions] = useState(settings);
  const [plans, setPlans] = useState(settings.plan_list.map(draft));
  const [selected, setSelected] = useState(settings.plan_list[0]?.plan_id ?? '');
  const [left, setLeft] = useState('');
  const [forecast, setForecast] = useState<NotoriousHuntGroupPreview | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const generation = useRef(0);
  useEffect(() => () => { generation.current++; }, []);
  useEffect(() => { generation.current++; setForecast(null); setBusy(false); }, [disabled]);
  const candidate = { ...options, plan_list: plans.map(item) };
  const changed = JSON.stringify(candidate) !== JSON.stringify(settings);
  const validPlans = plans.every(valid);
  const index = plans.findIndex(p => p.plan_id === selected);
  const current = plans[index];
  const invalidate = () => { generation.current++; setForecast(null); setError(''); setBusy(false); };
  const change = (next: Draft) => { invalidate(); setPlans(values => values.map((p, i) => i === index ? next : p)); };
  const estimate = async () => {
    const token = ++generation.current; setBusy(true); setError(''); setForecast(null);
    try {
      const result = await request({ type: 'preview_notorious_hunt', params: { instance_id: instanceId, group_id: groupId, revision, rewards_left: left === '' ? null : Number(left) } });
      if (token !== generation.current) return;
      if (result.type !== 'notorious_hunt_preview' || result.data.instance_id !== instanceId || result.data.group_id !== groupId || result.data.revision !== revision) throw new Error('估算来源已变化，请重新读取设置。');
      setForecast(result.data);
    } catch (e) { if (token === generation.current) setError(errorMessage(e)); }
    finally { if (token === generation.current) setBusy(false); }
  };
  return <Card aria-label="应用组恶名狩猎设置">
    <div className="card-title"><h2>恶名狩猎周常计划</h2><Badge appearance="outline">配置与只读估算</Badge></div>
    <p>每周免费奖励计划，与消耗电量的深度追猎分别配置。本阶段只保存及估算，挑战和领奖仍在开发。</p>
    <Field label="恶名狩猎开始星期"><Select value={options.weekly_challenge_start_weekday} disabled={disabled} onChange={(_, d) => { invalidate(); setOptions(v => ({ ...v, weekly_challenge_start_weekday: Number(d.value) })); }}>{days.map((name, i) => <option key={i} value={i + 1}>{name}</option>)}</Select></Field>
    <Checkbox label="恶名狩猎计划循环" checked={options.loop} disabled={disabled} onChange={(_, d) => { invalidate(); setOptions(v => ({ ...v, loop: d.checked === true })); }}/>
    <Field label="编辑恶名狩猎计划"><Select value={selected} disabled={disabled || !plans.length} onChange={(_, d) => setSelected(d.value)}>{!plans.length && <option value="">暂无计划</option>}{plans.map((p, i) => <option key={p.plan_id} value={p.plan_id}>{i + 1} · {p.mission_type_name}</option>)}</Select></Field>
    <div className="button-row">
      <Button disabled={disabled || plans.length >= 64} onClick={() => { invalidate(); const p = draft({ ...notoriousHuntItem, plan_id: crypto.randomUUID() }); setPlans(v => [...v, p]); setSelected(p.plan_id); }}>添加恶名狩猎计划</Button>
      <Button disabled={disabled || index < 0} onClick={() => { invalidate(); const next = plans.filter((_, i) => i !== index); setPlans(next); setSelected(next[Math.min(index, next.length - 1)]?.plan_id ?? ''); }}>删除恶名狩猎计划</Button>
      {([-1, 1] as const).map(move => <Button key={move} disabled={disabled || index < 0 || index + move < 0 || index + move >= plans.length} onClick={() => { invalidate(); const next = [...plans]; [next[index], next[index + move]] = [next[index + move], next[index]]; setPlans(next); }}>{move < 0 ? '上移恶名狩猎计划' : '下移恶名狩猎计划'}</Button>)}
    </div>
    {current && <>
      <Field label="恶名狩猎副本"><Select disabled={disabled} value={current.mission_type_name} onChange={(_, d) => change({ ...current, mission_type_name: d.value })}>{missions.map(m => <option key={m.mission_type_name} value={m.mission_type_name}>{m.mission_type_name_display ? `${m.mission_type_name} · ${m.mission_type_name_display}` : m.mission_type_name}</option>)}</Select></Field>
      <Field label="恶名狩猎等级"><Select disabled={disabled} value={current.level} onChange={(_, d) => change({ ...current, level: d.value })}>{chargePlanLevels.map(level => <option key={level}>{level}</option>)}</Select></Field>
      <Field label="恶名狩猎鸣徽"><Select disabled={disabled} value={current.notorious_hunt_buff_num} onChange={(_, d) => change({ ...current, notorious_hunt_buff_num: Number(d.value) })}>{[1, 2, 3].map(n => <option key={n} value={n}>第 {n} 项</option>)}</Select></Field>
      <div className="input-fields">{(['run_times', 'plan_times'] as const).map(key => <Field key={key} label={key === 'run_times' ? '恶名狩猎基线已运行次数' : '恶名狩猎目标次数'}><Input value={current[key]} inputMode="numeric" disabled={disabled} maxLength={7} onChange={(_, d) => change({ ...current, [key]: d.value })}/></Field>)}</div>
      <Field label="恶名狩猎预备编队"><Select value={current.predefined_team_idx} disabled={disabled} onChange={(_, d) => change({ ...current, predefined_team_idx: Number(d.value) })}><option value={-1}>不切换，使用当前队伍</option>{teams.teams.map((team, i) => <option key={i} value={i}>{i + 1} · {team.name}</option>)}</Select></Field>
      <Field label="恶名狩猎战斗脚本" hint={current.predefined_team_idx < 0 ? '保持当前队伍时采用此脚本。' : `采用编队脚本：${teams.teams[current.predefined_team_idx]?.auto_battle}；此名称保留。`}><Input disabled={disabled} value={current.auto_battle_config} onChange={(_, d) => change({ ...current, auto_battle_config: d.value })}/></Field>
    </>}
    {!validPlans && <p role="alert">基线次数须为 0..1000000 的整数，目标须为 0..1000 的整数，脚本须为有效名称。</p>}
    <Button disabled={disabled || !validPlans || !changed} onClick={() => { invalidate(); onSave(candidate); }}>保存恶名狩猎计划</Button>
    <details><summary>保存的周常计划</summary><NotoriousHuntSummary settings={settings}/></details>
    <p>按账号区服的游戏日估算。人工剩余次数只用于本次比较，留空表示未知；估算不会更新次数或周记录。</p>
    <Field label="估算剩余周奖励次数"><Input value={left} disabled={disabled} inputMode="numeric" maxLength={1} onChange={(_, d) => { invalidate(); setLeft(d.value); }}/></Field>
    {left !== '' && !integer(left, 3) && <p role="alert">剩余周奖励次数须为 0..3，或留空。</p>}
    <Button disabled={disabled || busy || changed || !validPlans || (left !== '' && !integer(left, 3))} onClick={() => void estimate()}>{busy ? '正在估算周常计划' : '估算保存的恶名狩猎计划'}</Button>
    {changed && <p>请先保存计划，再估算保存后的内容。</p>}{error && <p role="alert">{error}</p>}
    {forecast && <section aria-label="恶名狩猎估算结果"><p>游戏日 {forecast.forecast.game_day.date} · {days[forecast.forecast.weekday - 1]} · {decisions[forecast.forecast.decision]} · 基线扣除完整轮数 {forecast.forecast.rounds_rebased}。</p>
      <ol>{forecast.forecast.attempts.map((attempt, i) => <li key={i}>{settings.plan_list.find(p => p.plan_id === attempt.plan_id)?.mission_type_name} · 脚本 {attempt.script}</li>)}</ol>
      {forecast.forecast.pending_agent_plan_id && <p>代理人培养目标须在实际导航后确认，本次不预分配后续奖励。</p>}
    </section>}
  </Card>;
}
