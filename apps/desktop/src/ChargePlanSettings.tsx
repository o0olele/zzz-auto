import { useEffect, useRef, useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, Input, Select } from '@fluentui/react-components';
import { chargePlanDefaults, chargePlanCatalog, chargePlanAliases, chargePlanLevels, teamDefaultConfig, type ChargePlanConfig, type ChargePlanItem, type TeamConfig, type RestoreCharge, type ChargeGroupPreview } from './generated/protocol';
import { request, errorMessage } from './bridge';

const restoreNames: Record<RestoreCharge, string> = { none: '不使用', backup_only: '使用储蓄电量', ether_only: '使用以太电池', both: '允许两种来源' };
const targetName = (p: ChargePlanItem) => [p.category_name, p.mission_type_name, p.mission_name].filter(Boolean).join(' / ');
export function ChargePlanSummary({ settings = chargePlanDefaults }: { settings?: ChargePlanConfig }) {
  return <section aria-label="体力计划设置摘要"><p>体力计划：{settings.plan_list.length} 项 · 循环{settings.loop ? '开' : '关'} · 每日重置{settings.daily_reset_plan_times ? '开' : '关'} · 不足时跳过{settings.skip_plan ? '开' : '关'} · 恢复：{restoreNames[settings.restore_charge]} · 双倍{settings.double_reward ? '开' : '关'}。</p>
    <ol>{settings.plan_list.map(p => <li key={p.plan_id}>{targetName(p)} · 基线次数 {p.run_times}/{p.plan_times} · {p.predefined_team_idx < 0 ? `当前队伍 / ${p.auto_battle_config}` : `预备编队 ${p.predefined_team_idx + 1}`}</li>)}</ol>
    {settings.double_reward && <p>双倍临时目标：{targetName(settings.combat_simulation_double_reward_config)}。</p>}
  </section>;
}
type ItemDraft = Omit<ChargePlanItem, 'run_times' | 'plan_times'> & { run_times: string; plan_times: string };
const toDraft = (p: ChargePlanItem): ItemDraft => ({ ...p, run_times: String(p.run_times), plan_times: String(p.plan_times) });
const toItem = (p: ItemDraft): ChargePlanItem => ({ ...p, run_times: Number(p.run_times), plan_times: Number(p.plan_times) });
const integer = (s: string, max: number) => /^(0|[1-9]\d*)$/.test(s) && Number(s) <= max;
const validItem = (p: ItemDraft) => integer(p.run_times, 1000000) && integer(p.plan_times, 1000)
  && p.auto_battle_config.length > 0 && p.auto_battle_config.trim() === p.auto_battle_config
  && new TextEncoder().encode(p.auto_battle_config).length <= 240 && !['.', '..'].includes(p.auto_battle_config)
  && !/[/\\:\u0000-\u001f\u007f-\u009f]/u.test(p.auto_battle_config);

export function ChargePlanSettings({ settings = chargePlanDefaults, teams = teamDefaultConfig, instanceId, groupId, revision, disabled, onSave }: {
  settings?: ChargePlanConfig; teams?: TeamConfig; instanceId: number; groupId: string; revision: number; disabled: boolean; onSave: (settings: ChargePlanConfig) => void;
}) {
  const [options, setOptions] = useState(settings);
  const [plans, setPlans] = useState(settings.plan_list.map(toDraft));
  const [bonus, setBonus] = useState(toDraft(settings.combat_simulation_double_reward_config));
  const [selected, setSelected] = useState(settings.plan_list[0]?.plan_id ?? 'double');
  const [amounts, setAmounts] = useState({ charge: '', backup: '', ether: '', double: '' });
  const [forecast, setForecast] = useState<ChargeGroupPreview | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const generation = useRef(0);
  useEffect(() => () => { generation.current++; }, []);
  useEffect(() => { generation.current++; setForecast(null); setBusy(false); }, [disabled]);
  const candidate: ChargePlanConfig = { ...options, plan_list: plans.map(toItem), combat_simulation_double_reward_config: toItem(bonus) };
  const changed = JSON.stringify(candidate) !== JSON.stringify(settings);
  const valid = plans.every(validItem) && validItem(bonus);
  const index = plans.findIndex(p => p.plan_id === selected);
  const item = index < 0 ? bonus : plans[index];
  const category = chargePlanCatalog.find(c => c.category_name === item.category_name)!;
  const alias = chargePlanAliases.find(([categoryName, oldName]) => categoryName === item.category_name && oldName === item.mission_type_name);
  const mission = category.mission_type_list.find(m => m.mission_type_name === (alias?.[2] ?? item.mission_type_name));
  const invalidate = () => { generation.current++; setForecast(null); setError(''); setBusy(false); };
  const changeItem = (next: ItemDraft) => { invalidate(); if (index < 0) setBonus(next); else setPlans(current => current.map((p, i) => i === index ? next : p)); };
  const toggle = (key: 'loop' | 'daily_reset_plan_times' | 'skip_plan' | 'double_reward', value: boolean) => { invalidate(); setOptions(c => ({ ...c, [key]: value })); };
  const resourceEmpty = !amounts.charge && !amounts.backup && !amounts.ether;
  const resourceValid = resourceEmpty || [amounts.charge, amounts.backup, amounts.ether].every(v => integer(v, 1000000));
  const bonusValid = !amounts.double || integer(amounts.double, 5);
  const estimate = async () => {
    const token = ++generation.current; setBusy(true); setError(''); setForecast(null);
    try {
      const result = await request({ type: 'preview_charge_plan', params: { instance_id: instanceId, group_id: groupId, revision, resources: resourceEmpty ? null : { charge: Number(amounts.charge), backup: Number(amounts.backup), ether: Number(amounts.ether) }, double_reward_left: amounts.double ? Number(amounts.double) : null } });
      if (generation.current !== token) return;
      if (result.type !== 'charge_plan_preview' || result.data.revision !== revision || result.data.instance_id !== instanceId || result.data.group_id !== groupId) throw new Error('估算来源已变化，请重新读取设置。');
      setForecast(result.data);
    } catch (e) { if (generation.current === token) setError(errorMessage(e)); }
    finally { if (generation.current === token) setBusy(false); }
  };
  const decisions = { disabled: '目标为零，不执行', completed: '本轮已完成', ready: '电量预估足够', needs_restore: '需在副本内核对并恢复', insufficient: '电量及允许的恢复来源不足', unknown_resources: '资源未知，等待读取' };
  return <Card aria-label="应用组体力计划设置">
    <div className="card-title"><h2>体力刷本计划</h2><Badge appearance="outline">配置与只读估算</Badge></div>
    <p>副本执行仍在开发。可编辑和估算保存计划，当前不会进入副本、恢复电量或消耗物品。旧次数及手工次数只作为计划基线。</p>
    <Checkbox label="体力计划循环" checked={options.loop} disabled={disabled} onChange={(_, d) => toggle('loop', d.checked === true)}/>
    <Checkbox label="体力计划每日重置" checked={options.daily_reset_plan_times} disabled={disabled} onChange={(_, d) => toggle('daily_reset_plan_times', d.checked === true)}/>
    <Checkbox label="体力不足时跳过计划" checked={options.skip_plan} disabled={disabled} onChange={(_, d) => toggle('skip_plan', d.checked === true)}/>
    <Checkbox label="体力计划优先双倍活动" checked={options.double_reward} disabled={disabled} onChange={(_, d) => toggle('double_reward', d.checked === true)}/>
    <Field label="体力恢复来源"><Select value={options.restore_charge} disabled={disabled} onChange={(_, d) => { invalidate(); setOptions(c => ({ ...c, restore_charge: d.value as RestoreCharge })); }}>{Object.entries(restoreNames).map(([v, name]) => <option key={v} value={v}>{name}</option>)}</Select></Field>
    {settings.last_daily_reset_day && <p>已保存重置日：{settings.last_daily_reset_day.date} · {settings.last_daily_reset_day.region}。仅运行流程可更新。</p>}
    <Field label="编辑体力计划"><Select value={selected} disabled={disabled} onChange={(_, d) => setSelected(d.value)}>
      {plans.map((p, i) => <option key={p.plan_id} value={p.plan_id}>{i + 1} · {targetName(toItem(p))}</option>)}<option value="double">双倍活动临时目标</option>
    </Select></Field>
    <div className="button-row">
      <Button disabled={disabled || plans.length >= 64} onClick={() => { invalidate(); const p = toDraft({ ...chargePlanDefaults.combat_simulation_double_reward_config, plan_id: crypto.randomUUID() }); setPlans(c => [...c, p]); setSelected(p.plan_id); }}>添加体力计划</Button>
      <Button disabled={disabled || index < 0} onClick={() => { invalidate(); const next = plans.filter((_, i) => i !== index); setPlans(next); setSelected(next[Math.min(index, next.length - 1)]?.plan_id ?? 'double'); }}>删除体力计划</Button>
      {([-1, 1] as const).map(move => <Button key={move} disabled={disabled || index < 0 || index + move < 0 || index + move >= plans.length} onClick={() => { invalidate(); const next = [...plans]; [next[index], next[index + move]] = [next[index + move], next[index]]; setPlans(next); }}>{move < 0 ? '上移体力计划' : '下移体力计划'}</Button>)}
    </div>
    <Field label="体力副本分类"><Select value={item.category_name} disabled={disabled || index < 0} onChange={(_, d) => { const c = chargePlanCatalog.find(c => c.category_name === d.value)!; const m = c.mission_type_list[0]; changeItem({ ...item, category_name: c.category_name, mission_type_name: m?.mission_type_name ?? '', mission_name: m?.mission_list[0]?.mission_name ?? null }); }}>{chargePlanCatalog.map(c => <option key={c.category_name}>{c.category_name}</option>)}</Select></Field>
    <Field label="体力副本类型"><Select value={item.mission_type_name} disabled={disabled || !category.mission_type_list.length} onChange={(_, d) => { const selectedAlias = chargePlanAliases.find(([categoryName, oldName]) => categoryName === item.category_name && oldName === d.value); const m = category.mission_type_list.find(m => m.mission_type_name === (selectedAlias?.[2] ?? d.value))!; changeItem({ ...item, mission_type_name: d.value, mission_name: m.mission_list[0]?.mission_name ?? null }); }}>{alias && <option value={alias[1]}>{alias[1]}（现名：{alias[2]}）</option>}{category.mission_type_list.length ? category.mission_type_list.map(m => <option key={m.mission_type_name} value={m.mission_type_name}>{m.mission_type_name_display ? `${m.mission_type_name} · ${m.mission_type_name_display}` : m.mission_type_name}</option>) : <option value="">无副本类型</option>}</Select></Field>
    <Field label="体力关卡"><Select value={item.mission_name ?? ''} disabled={disabled || !mission?.mission_list.length} onChange={(_, d) => changeItem({ ...item, mission_name: d.value || null })}>{mission?.mission_list.length ? mission.mission_list.map(m => <option key={m.mission_name} value={m.mission_name}>{m.mission_name_display ? `${m.mission_name} · ${m.mission_name_display}` : m.mission_name}</option>) : <option value="">无下级关卡</option>}</Select></Field>
    <div className="input-fields">{(['run_times', 'plan_times'] as const).map(key => <Field key={key} label={key === 'run_times' ? '体力计划基线已运行次数' : '体力计划目标次数'}><Input value={item[key]} inputMode="numeric" maxLength={7} disabled={disabled || index < 0} onChange={(_, d) => changeItem({ ...item, [key]: d.value })}/></Field>)}</div>
    <Field label="体力模拟室卡片数量"><Select value={String(item.card_num ?? '')} disabled={disabled || item.category_name !== '实战模拟室'} onChange={(_, d) => changeItem({ ...item, card_num: d.value ? Number(d.value) : null })}><option value="">默认数量</option>{[1,2,3,4,5].map(n => <option key={n} value={n}>{n} 张</option>)}</Select></Field>
    <Field label="体力预备编队"><Select value={item.predefined_team_idx} disabled={disabled} onChange={(_, d) => changeItem({ ...item, predefined_team_idx: Number(d.value) })}><option value={-1}>不切换，使用当前队伍</option>{teams.teams.map((t, i) => <option key={i} value={i}>{i + 1} · {t.name}</option>)}</Select></Field>
    <Field label="体力战斗脚本" hint={item.predefined_team_idx < 0 ? '不切换编队时采用此脚本。' : `采用所选编队的脚本：${teams.teams[item.predefined_team_idx]?.auto_battle}；此名称保留。`}><Input value={item.auto_battle_config} disabled={disabled} onChange={(_, d) => changeItem({ ...item, auto_battle_config: d.value })}/></Field>
    <Field label="体力深度追猎等级"><Select value={item.level} disabled={disabled || item.category_name !== '恶名狩猎'} onChange={(_, d) => changeItem({ ...item, level: d.value })}>{chargePlanLevels.map(level => <option key={level}>{level}</option>)}</Select></Field>
    <Field label="体力深度追猎鸣徽"><Select value={item.notorious_hunt_buff_num} disabled={disabled || item.category_name !== '恶名狩猎'} onChange={(_, d) => changeItem({ ...item, notorious_hunt_buff_num: Number(d.value) })}>{[1,2,3].map(n => <option key={n} value={n}>第 {n} 项</option>)}</Select></Field>
    {!valid && <p role="alert">次数须为整数：已运行 0..1000000、目标 0..1000。脚本名称须有效，且不能包含路径。</p>}
    <Button disabled={disabled || !valid || !changed} onClick={() => { invalidate(); onSave(candidate); }}>保存体力计划</Button>
    <details><summary>保存的计划</summary><ChargePlanSummary settings={settings}/></details>
    <p>人工资源估算：留空表示未知；填写时须填齐三项。每项独立比较当前资源，不预扣后续电量；估算不保存或修改次数。</p>
    <div className="input-fields">{([['charge','估算当前电量'],['backup','估算储蓄电量'],['ether','估算以太电池'],['double','估算双倍剩余次数']] as const).map(([key, label]) => <Field key={key} label={label}><Input value={amounts[key]} inputMode="numeric" maxLength={7} disabled={disabled} onChange={(_, d) => { invalidate(); setAmounts(c => ({ ...c, [key]: d.value })); }}/></Field>)}</div>
    {!resourceValid && <p role="alert">资源须全部留空，或填齐三个 0..1000000 的整数。</p>}{!bonusValid && <p role="alert">双倍剩余次数须为 0..5。</p>}
    <Button disabled={disabled || busy || changed || !valid || !resourceValid || !bonusValid} onClick={() => void estimate()}>{busy ? '正在估算体力计划' : '估算保存的体力计划'}</Button>
    {changed && <p>请先保存计划，再估算保存后的内容。</p>}{error && <p role="alert">{error}</p>}
    {forecast && <section aria-label="体力计划估算结果"><p>游戏日 {forecast.forecast.game_day.date} · {forecast.forecast.reset_daily ? '本次按每日重置计算' : '保留基线次数'} · 已扣除完整轮数 {forecast.forecast.rounds_rebased}。</p>
      {forecast.forecast.double_check_pending && <p>双倍次数未知，尚未形成临时计划。</p>}{forecast.forecast.temporary && <p>优先临时双倍计划：{targetName(forecast.forecast.temporary)} · {forecast.forecast.temporary.card_num} 张 · {forecast.forecast.temporary_script}。</p>}
      <ol>{forecast.forecast.items.map(p => <li key={p.plan_id}>{targetName(forecast.forecast.config.plan_list.find(i => i.plan_id === p.plan_id)!)} · 有效已运行 {p.effective_run_times} · 预估 {p.estimated_charge} 电量 · {decisions[p.decision]} · 脚本 {p.script}{p.plan_id === forecast.forecast.next_plan_id ? ' · 常规下一项' : ''}</li>)}</ol>
    </section>}
  </Card>;
}
