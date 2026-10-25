import { useState } from 'react';
import { Badge, Button, Card, Field, Input, Select } from '@fluentui/react-components';
import { worldPatrolDefaults, worldPatrolRouteLists, worldPatrolRoutes, type WorldPatrolConfig, type GroupPlan, type DailyProgress } from './generated/protocol';

function selectedRoutes(name: string) {
  if (!name) return [...worldPatrolRoutes];
  const list = worldPatrolRouteLists.find(item => item.name === name);
  return list ? worldPatrolRoutes.filter(route => (list.route_items as readonly string[]).includes(route.id) === (list.list_type === 'whitelist')) : [];
}
export function WorldPatrolSettings({ settings = worldPatrolDefaults, disabled, onSave }: {
  settings?: WorldPatrolConfig; disabled: boolean; onSave: (settings: WorldPatrolConfig) => void;
}) {
  const [draft, setDraft] = useState(settings);
  const routes = selectedRoutes(draft.route_list);
  const name = draft.auto_battle;
  const valid = name.length > 0 && name.trim() === name && new TextEncoder().encode(name).length <= 240 && !['.', '..'].includes(name) && !/[/\\:\u0000-\u001f\u007f-\u009f]/u.test(name)
    && routes.length > 0 && Number.isInteger(draft.daily_loop_count) && draft.daily_loop_count >= 1 && draft.daily_loop_count <= 48
    && Number.isInteger(draft.loop_interval_seconds) && draft.loop_interval_seconds >= 0 && draft.loop_interval_seconds <= 86400
    && Number.isInteger(draft.route_retry_times) && draft.route_retry_times >= 0 && draft.route_retry_times <= 10
    && Number.isInteger(draft.ui_disappear_seconds) && draft.ui_disappear_seconds >= 1 && draft.ui_disappear_seconds <= 999
    && Number.isInteger(draft.timeout_ms) && draft.timeout_ms >= 60000 && draft.timeout_ms <= 86400000;
  const changed = JSON.stringify(draft) !== JSON.stringify(settings);
  return <Card aria-label="应用组锄大地设置">
    <div className="card-title"><h2>锄大地</h2><Badge appearance="outline">按账号与应用组保存</Badge></div>
    <p>按已有路线传送、跑图和自动战斗。当前提供 {worldPatrolRoutes.length} 条路线和 {worldPatrolRouteLists.length} 份名单，真实游戏流程待验收。</p>
    <Field label="锄大地路线名单" hint={`本轮 ${routes.length} 条路线；名单按旧版顺序执行。`}>
      <Select disabled={disabled} value={draft.route_list} onChange={(_, data) => setDraft(current => ({ ...current, route_list: data.value }))}>
        <option value="">全部路线</option>
        {worldPatrolRouteLists.map(list => <option key={list.name} value={list.name}>{list.name}</option>)}
      </Select>
    </Field>
    <details><summary>查看本轮路线</summary><ul>{routes.map(route => <li key={route.id}>{route.tp_name} · {route.id} · {route.points.length} 个行走点</li>)}</ul></details>
    <Field label="锄大地战斗脚本" hint="沿用当前账号的战斗输入与识别设置；开始前冻结所选脚本。">
      <Input disabled={disabled} value={draft.auto_battle} onChange={(_, data) => setDraft(current => ({ ...current, auto_battle: data.value }))}/>
    </Field>
    <div className="form-grid">
      <Field label="锄大地每天循环次数"><Input type="number" min={1} max={48} disabled={disabled} value={String(draft.daily_loop_count)} onChange={(_, data) => setDraft(current => ({ ...current, daily_loop_count: Number(data.value) }))}/></Field>
      <Field label="锄大地轮次起点间隔（秒）" hint="默认 1800 秒；扣除跑图耗时后等待，等待前返回录像店。"><Input type="number" min={0} max={86400} disabled={disabled} value={String(draft.loop_interval_seconds)} onChange={(_, data) => setDraft(current => ({ ...current, loop_interval_seconds: Number(data.value) }))}/></Field>
      <Field label="锄大地单条路线重试次数"><Input type="number" min={0} max={10} disabled={disabled} value={String(draft.route_retry_times)} onChange={(_, data) => setDraft(current => ({ ...current, route_retry_times: Number(data.value) }))}/></Field>
      <Field label="锄大地重试后卡住处理"><Select disabled={disabled} value={draft.route_retry_action} onChange={(_, data) => setDraft(current => ({ ...current, route_retry_action: data.value as WorldPatrolConfig['route_retry_action'] }))}><option value="skip_on_stuck_again">再次卡住时跳过路线</option><option value="retry_on_stuck_again">再次尝试脱困</option></Select></Field>
      <Field label="锄大地界面消失判定（秒）"><Input type="number" min={1} max={999} disabled={disabled} value={String(draft.ui_disappear_seconds)} onChange={(_, data) => setDraft(current => ({ ...current, ui_disappear_seconds: Number(data.value) }))}/></Field>
      <Field label="锄大地总超时（分钟）" hint="包括轮间等待，暂停期间不计时。"><Input type="number" min={1} max={1440} disabled={disabled} value={String(draft.timeout_ms / 60000)} onChange={(_, data) => setDraft(current => ({ ...current, timeout_ms: Number(data.value) * 60000 }))}/></Field>
    </div>
    <p>失去位置时停止前进；界面消失时结束任务。当前不支持异常后自动重开并登录游戏。每天完成记录只复用同一组、相同设置的新版记录。</p>
    {!valid && <p role="alert">请填写有效脚本名称、路线和数值范围。</p>}
    <Button disabled={disabled || !valid || !changed} onClick={() => onSave(draft)}>保存锄大地设置</Button>
  </Card>;
}
export function WorldPatrolPlan({ plan }: { plan: GroupPlan }) {
  const settings = plan.world_patrol_config ?? worldPatrolDefaults;
  const saved = plan.world_patrol_checkpoint;
  return <section aria-label="锄大地运行计划"><p>锄大地：{settings.route_list || '全部路线'} · 每轮 {selectedRoutes(settings.route_list).length} 条路线 · {settings.daily_loop_count} 轮 · 轮次起点间隔 {settings.loop_interval_seconds} 秒 · 脚本 {plan.world_patrol_combat?.config.script ?? settings.auto_battle}</p>
    {saved && <p>本游戏日已完成 {saved.completed_rounds} 轮，本轮已完成 {saved.finished.length} 条路线。{saved.completed_rounds >= settings.daily_loop_count ? '当天目标已完成。' : '将继续剩余路线与轮次。'}</p>}
    <p>跑图使用前台键鼠，战斗采用当前账号的输入设置；失败路线不计入完成轮数。</p></section>;
}
const stages: Record<string, string> = { Inspect: '确认画面', TrackSelect: '打开绳网', TrackStop: '停止任务追踪', TrackReturn: '返回大世界', OpenMap: '打开 3D 地图', Area: '选择区域', SubOpen: '展开子区域', SubArea: '选择子区域', Filter: '打开筛选', FilterOption: '筛选传送点', CloseFilter: '关闭筛选', Zoom: '调整地图缩放', Search: '查找传送点', Confirm: '确认目标传送点', Arrival: '等待传送', Move: '按路线行走', Combat: '自动战斗', Leave: '离开勘域', LeaveArea: '选择录像店', LeavePoint: '选择房间', LeaveConfirm: '确认返回', LeaveArrival: '等待返回录像店', Wait: '轮间等待', Done: '已完成' };
export function WorldPatrolStatus({ progress }: { progress: DailyProgress }) {
  const patrol = progress.world_patrol;
  if (!patrol) return null;
  const route = patrol.routes[patrol.route_index];
  const remaining = patrol.next_round_ms == null ? 0 : Math.max(0, Math.ceil((patrol.next_round_ms - progress.active_ms) / 1000));
  return <section aria-label="锄大地运行进度"><p>锄大地 · {stages[progress.stage] ?? progress.stage} · 完成 {patrol.completed_rounds}/{patrol.config.daily_loop_count} 轮</p>
    <p>本轮完成 {patrol.finished.length}/{patrol.routes.length} 条路线 · 失败 {patrol.failed.length} 条{route ? ` · ${route.tp_name} · 行走点 ${patrol.point_index}/${route.points.length} · 已重试 ${patrol.attempt} 次` : ''}</p>
    {progress.stage === 'Wait' && <p>下一轮还需等待约 {remaining} 秒</p>}</section>;
}
