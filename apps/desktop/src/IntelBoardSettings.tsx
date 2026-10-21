import { useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, Input, Select } from '@fluentui/react-components';
import { teamDefaultConfig, type DailyProgress, type GroupPlan, type IntelBoardConfig, type TeamConfig } from './generated/protocol';

const combatModes: Record<string, string> = { foreground: '前台键鼠', foreground_legacy: '前台键鼠（兼容）', background: '后台键鼠', background_post: '后台键鼠（异步）', xbox: 'Xbox 360', ds4: 'DualShock 4' };
const stages: Record<string, string> = { Inspect: '确认画面', OpenMap: '打开地图', SelectArea: '选择城区', SelectPoint: '选择房间', ConfirmTransport: '确认传送', WaitArrival: '等待到达', OpenGuide: '打开功能导览', OpenBoard: '打开情报板', ReadBoard: '读取周进度', OpenFilter: '打开筛选', ResetFilter: '重置筛选', SelectNotorious: '选择恶名狩猎', SelectExpert: '选择专业挑战室', CloseFilter: '确认筛选', Refresh: '刷新委托', FindCommission: '寻找委托', Scroll: '翻页寻找', OpenCommission: '打开委托', Accept: '接取委托', Next: '确认挑战', ConfirmFailed: '处理接取失败', OpenTeams: '打开编队', SelectTeam: '选择预备编队', Deploy: '出战', WaitDeploy: '等待战斗画面', ConfirmNoAgents: '处理空队伍', Move: '战前移动', Combat: '自动战斗', Settlement: '返回情报板结算', Return: '返回大世界', Done: '本次已结束' };
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
    <p>按区服的游戏周保存进度。普通模式以贡献达到 1000 为目标；经验模式按恶名狩猎每次 500、专业挑战室每次 250 累计到 5000。可独立或按组试运行，实机验收保留。</p>
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

export function IntelBoardPlan({ plan }: { plan: GroupPlan }) {
  const combat = plan.app_combat?.config;
  const totals = plan.intel_board_history?.totals;
  return <section aria-label="情报板运行计划">
    <p>情报板：{intelBoardSummary(plan.intel_board_config)} · 录像店 → 房间</p>
    {totals && <p>本周贡献 {totals.contribution}/1000 · 恶名狩猎 {totals.notorious_hunt_count} 次 · 专业挑战室 {totals.expert_challenge_count} 次 · 估计经验 {totals.estimated_base_exp + totals.notorious_hunt_count * 500 + totals.expert_challenge_count * 250}/5000</p>}
    {combat && <p>冻结战斗脚本：{combat.script} · 输入 {combatModes[combat.mode] ?? combat.mode} · {combat.directml_device == null ? 'CPU' : `DirectML ${combat.directml_device}`} · 总超时 {combat.timeout_ms} ms</p>}
    <p>仅已确认完成并返回板面的委托增加次数；当前无委托会结束本次运行，后续仍可继续周目标。</p>
  </section>;
}

export function IntelBoardStatus({ progress }: { progress: DailyProgress }) {
  const ledger = progress.intel_board;
  if (!ledger) return null;
  const hunt = ledger.baseline.notorious_hunt_count + ledger.settled.filter(c => c.kind === 'notorious_hunt' && c.result === 'completed').length;
  const expert = ledger.baseline.expert_challenge_count + ledger.settled.filter(c => c.kind === 'expert_challenge' && c.result === 'completed').length;
  const exp = ledger.estimated_base_exp + hunt * 500 + expert * 250;
  return <section aria-label="情报板运行进度">
    <p>情报板 · 游戏周 {ledger.week.monday} · 阶段 {stages[progress.stage] ?? '确认当前画面'}</p>
    <p>贡献 {ledger.observed_contribution ?? ledger.baseline.contribution}/1000 · 恶名狩猎 {hunt} 次 · 专业挑战室 {expert} 次 · 估计经验 {exp}/5000</p>
    {ledger.pending && <p>当前{ledger.pending.kind === 'notorious_hunt' ? '恶名狩猎' : '专业挑战室'}：{ledger.pending.result ? '战斗结果已确认，等待返回板面' : ledger.pending.deployed ? '已投递出战' : '接取中'}</p>}
    {progress.intel_battle && <p>战斗脚本 {progress.intel_battle.config.script} · 已识别 {progress.intel_battle.frames} 帧 · {progress.intel_battle.inputs_released ? '战斗输入已释放' : '战斗输入处理中'}</p>}
    {progress.intel_flow?.outcome && <p>{progress.intel_flow.outcome === 'weekly_goal' ? '本周目标已完成' : '当前无可代行委托，周目标仍未完成'}</p>}
  </section>;
}
