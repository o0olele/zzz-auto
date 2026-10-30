import { WorldPatrolPlan } from './WorldPatrolSettings';
import { SuibianTempleSummary } from './SuibianTempleSettings';
import { ChargePlanSummary } from './ChargePlanSettings';
import { LostVoidSummary } from './LostVoidSettings';
import { RedemptionResults, RedemptionStatus } from './RedemptionSettings';
import { IntelBoardPlan, IntelBoardStatus } from './IntelBoardSettings';
import { dismantleSummary } from './DismantleSettings';
import { RandomPlayStatus, randomPlaySummary } from './RandomPlaySettings';
import { TeamSummary, TeamCheckStatus } from './TeamSettings';
import { useEffect, useRef, useState } from 'react';
import { Badge, Button, Card, Checkbox, MessageBar, MessageBarBody, Spinner } from '@fluentui/react-components';
import type { Command, GroupPlan, RunHistory, Snapshot } from './generated/protocol';
import { gameRegionOptions } from './generated/protocol';
import { errorMessage, request } from './bridge';
import { coffeeSummary } from './CoffeeSettings';
import { dailySummary } from './DailySettings';
import { afterDoneName } from './AfterDoneBanner';
import { GroupRunStatus, isRunActive, RunSaveRecovery, RunSteps, runStateNames as stateNames } from './GroupRunStatus';

const availability = { lost_void_trial: '迷失之地试运行（待实机验收）', charge_trial: '体力试运行（待实机验收）', mail_trial: '邮件试运行', coffee_trial: '咖啡试运行（待实机验收）', daily_trial: '日常试运行（待实机验收）', unmigrated: '尚无执行器', unknown: '未注册' };

export function GroupRuns({ instanceId, groupId, revision, snapshot, connected, blocked, onCommand, showStatus = true, appIds, rerunCompleted = false, standaloneAppId }: {
  instanceId: number; groupId: string; revision: number; snapshot: Snapshot | null;
  connected: boolean; blocked: boolean; onCommand: (command: Command) => Promise<void>; showStatus?: boolean; appIds?: string[]; rerunCompleted?: boolean; standaloneAppId?: string;
}) {
  const standalone = standaloneAppId !== undefined;
  const [plan, setPlan] = useState<GroupPlan | null>(null);
  const appCombat = plan?.app_combat ?? plan?.world_patrol_combat ?? plan?.exploration_combat ?? plan?.charge_combat ?? plan?.lost_void_combat;
  const [history, setHistory] = useState<RunHistory | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [planning, setPlanning] = useState(false);
  const [historyBusy, setHistoryBusy] = useState(false);
  const [planError, setPlanError] = useState('');
  const [historyError, setHistoryError] = useState('');
  const currentRun = snapshot?.run;
  const active = isRunActive(currentRun);
  const run = currentRun?.context?.instance_id === instanceId && currentRun.context.group_id === groupId ? currentRun : null;
  const selectionKey = appIds === undefined ? 'saved' : JSON.stringify(appIds);
  const key = `${instanceId}/${groupId}/${revision}/${selectionKey}/${rerunCompleted}/${standaloneAppId}/${connected}/${snapshot?.window?.hwnd}/${snapshot?.window?.pid}/${snapshot?.capture_method}/${currentRun?.id}/${currentRun?.state}/${currentRun?.finished_at}`;
  const requestKey = `${key}/${snapshot?.record_error}/${blocked}`;
  const currentKey = useRef(requestKey); currentKey.current = requestKey;
  const planRequest = useRef(0);

  useEffect(() => { setPlan(null); setConfirmed(false); setPlanError(''); }, [key]);
  useEffect(() => {
    planRequest.current++;
    setConfirmed(false); setPlanning(false);
    return () => { planRequest.current++; };
  }, [requestKey]);
  useEffect(() => {
    if (!plan) return;
    const remaining = new Date(plan.valid_until).getTime() - Date.now();
    const timer = window.setTimeout(() => { setPlan(null); setConfirmed(false); setPlanError('预览的游戏日已过期，请重新预览并确认。'); }, Math.max(0, remaining));
    return () => window.clearTimeout(timer);
  }, [plan]);

  async function preview() {
    const requested = requestKey;
    const requestId = ++planRequest.current;
    setPlanning(true); setPlanError(''); setConfirmed(false); setPlan(null);
    try {
      const value = await request(standalone ? { type: 'preview_standalone', params: { instance_id: instanceId, group_id: groupId, app_id: standaloneAppId } } : { type: 'preview_group', params: { instance_id: instanceId, group_id: groupId, ...(appIds === undefined ? {} : { app_ids: appIds }), ...(rerunCompleted ? { rerun_completed: true } : {}) } });
      if (currentKey.current !== requested || planRequest.current !== requestId) return;
      if (value.type !== 'group_plan') throw new Error('引擎未返回应用组预览');
      if (value.data.context.config_revision !== revision) throw new Error('配置已更新，请先读取最新账号配置后重新预览。');
      if (value.data.context.instance_id !== instanceId || value.data.context.group_id !== groupId) throw new Error('返回的计划与所选账号或应用组不符，请重新预览。');
      if ((value.data.context.standalone ?? false) !== standalone || (standalone && (value.data.after_done !== 'none' || value.data.context.app_ids.length !== 1 || value.data.context.app_ids[0] !== standaloneAppId))) throw new Error('返回的计划与独立运行选项不符，请重新预览。');
      if ((value.data.context.rerun_completed ?? false) !== rerunCompleted) throw new Error('返回的计划与本次重新运行选项不符，请重新预览。');
      if (appIds !== undefined && (value.data.context.app_ids.length !== appIds.length || new Set(value.data.context.app_ids).size !== appIds.length || value.data.context.app_ids.some(id => !appIds.includes(id)))) throw new Error('返回的计划与本次所选应用不符，请重新预览。');
      setPlan(value.data);
    } catch (reason) { if (currentKey.current === requested && planRequest.current === requestId) setPlanError(errorMessage(reason)); }
    finally { if (currentKey.current === requested && planRequest.current === requestId) setPlanning(false); }
  }

  useEffect(() => {
    let alive = true;
    setHistory(null); setHistoryError('');
    if (connected) {
      setHistoryBusy(true);
      request({ type: 'run_history', params: { instance_id: instanceId } }).then(value => {
        if (alive && value.type === 'run_history') setHistory(value.data);
      }).catch(reason => { if (alive) setHistoryError(errorMessage(reason)); }).finally(() => { if (alive) setHistoryBusy(false); });
    }
    return () => { alive = false; };
  }, [connected, instanceId, currentRun?.id, currentRun?.state, currentRun?.finished_at, snapshot?.record_error]);

  async function refreshHistory() {
    setHistoryBusy(true); setHistoryError('');
    const requested = requestKey;
    try {
      const value = await request({ type: 'run_history', params: { instance_id: instanceId } });
      if (currentKey.current === requested && value.type === 'run_history') setHistory(value.data);
    } catch (reason) { if (currentKey.current === requested) setHistoryError(errorMessage(reason)); }
    finally { setHistoryBusy(false); }
  }

  const disabled = !connected || blocked || planning || appIds?.length === 0;
  return <>
    <Card className="group-run-card"><div className="card-title"><h2>{standalone ? '独立应用试运行' : '应用组试运行'}</h2><Badge appearance="outline">日常试运行</Badge></div>
      {standalone ? <p>只执行本次选择的一个应用，运行参数、输入收尾与每日证明由引擎统一处理。咖啡完整流程待实机验收，请确认当天仍可喝咖啡。</p> : <p>按保存顺序执行{appIds === undefined ? '启用项' : '本次所选应用，不改写应用组配置'}。邮件参数在通用设置中保存，咖啡参数按账号和组保存；咖啡从大世界自动传送并点单，完整流程待实机验收。请确认当天仍可喝咖啡。</p>}
      {appIds?.length === 0 && <p>请至少选择一个本次应用。</p>}
      <Button disabled={disabled} onClick={() => void preview()}>预览运行计划</Button>
      {planning && <Spinner size="tiny" label="检查应用组"/>}
      {planError && <MessageBar intent="error"><MessageBarBody>{planError}</MessageBarBody></MessageBar>}
      {plan && <div aria-label="应用组运行计划">
        <p>{plan.context.instance_name} · {plan.context.group_name ?? plan.context.group_id} · 修订 {plan.context.config_revision}</p>
        <p>结束后：{afterDoneName(plan.after_done)}{plan.after_done && plan.after_done !== 'none' ? ' · 正常完成后等待 60 秒，可取消' : ''}</p>
        <p>{gameRegionOptions.find(option => option.value === plan.game_day.region)?.label} · 游戏日 {plan.game_day.date} · {plan.run_policy === 'always' ? '每次运行' : '跳过本游戏日已完成'}</p>
        {plan.context.rerun_completed && <p>仅本次：重新运行已完成应用，长期运行策略保留。</p>}
        <p>启动 / 继续延时 {plan.mail_config.start_delay_ms} ms · 邮件超时 {plan.mail_config.timeout_ms} ms</p>
        {plan.items.some(item => item.app_id === 'coffee' && item.enabled) && <p>咖啡：{coffeeSummary(plan.coffee_config)} · 澄辉坪 → 汀曼咖啡</p>}
        {plan.items.some(item => item.app_id === 'charge_plan' && item.enabled) && <ChargePlanSummary settings={plan.charge_plan_config}/>}
        {plan.items.some(item => item.app_id === 'map_exploration' && item.enabled) && <section aria-label="空洞探索运行计划"><p>空洞探索：从当前区域开始，自动将地图缩放至最小并拖图扫描，先解锁观测奇点，再开启小卡格车；沿小地图绿色引导点行走，遇战先战斗。完成须由区域信息中的小卡格车计数确认。</p><p>战斗脚本：{plan.exploration_combat?.config.script} · 输入方式：{plan.exploration_combat?.config.mode} · 总超时 30 分钟。战斗配置在自动战斗页保存。</p></section>}
        {plan.items.some(item => item.app_id === 'lost_void' && item.enabled) && <LostVoidSummary settings={plan.lost_void_config}/>}
        {plan.lost_void_planning && <p>迷失之地只读进度：当日通关 {plan.lost_void_planning.progress.daily_run_times} 次 · 本周通关 {plan.lost_void_planning.progress.weekly_run_times} 次 · 待领取结算 {plan.lost_void_planning.pending_claims.length} 份。启动后会补交原记录并重新核对。</p>}
        {plan.charge_combat && <p>体力战斗采用账号输入设置：{plan.charge_combat.config.mode} · {plan.charge_combat.config.directml_device == null ? 'CPU' : `DirectML ${plan.charge_combat.config.directml_device}`}。全部启用计划的脚本已预检，按各计划或预备编队选择。</p>}
        {plan.items.some(item => item.app_id === 'world_patrol' && item.enabled) && <WorldPatrolPlan plan={plan}/>}
        {plan.items.some(item => item.app_id === 'intel_board' && item.enabled) && <IntelBoardPlan plan={plan}/>}
        {plan.items.some(item => item.app_id === 'suibian_temple' && item.enabled) && <><SuibianTempleSummary settings={plan.suibian_temple_config}/><p>澄辉坪 → 随便观 · 总活动时间包括全部经营子流程。已开启的购买将消耗玩法货币。</p></>}
        {plan.items.some(item => item.availability === 'daily_trial' && item.enabled) && <p>日常：{dailySummary(plan.daily_config)} · 刮刮卡：六分街 → 报刊亭；卦象：澄辉坪 → 阿朔；饼铺：布亚斯特城区 → 吼吼饼铺。三项共享每日领取次数。</p>}
        {plan.items.some(item => item.app_id === 'drive_disc_dismantle' && item.enabled) && <p>驱动盘拆解：{dismantleSummary(plan.dismantle_config)} · 所选驱动盘将被消耗。</p>}
        {plan.items.some(item => item.app_id === 'random_play' && item.enabled) && <p>录像店经营：{randomPlaySummary(plan.random_play_config)} · 按本次游戏日期轮换宣传员，已有营业则返回。</p>}
        {plan.items.some(item => item.app_id === 'predefined_team_checker' && item.enabled) && <div>
          <p>预备编队检查：总超时 {plan.predefined_teams?.check_timeout_ms ?? 300000} ms，扫描五页并在返回大世界后保存可靠成员。未识别或冲突的编队保留原值。</p>
          <TeamSummary settings={plan.predefined_teams}/>
        </div>}
        {plan.items.some(item => item.app_id === 'redemption_code' && item.enabled) && <section aria-label="兑换码计划">
          <p>待兑换有效码：{plan.redemption_codes?.length ?? 0} 个。按账号跳过已处理码；逐码结果跨游戏日保存。</p>
          <ul>{plan.redemption_codes?.map(code => <li key={code.code}>{code.code} · 有效至 {code.expires_on}</li>)}</ul>
          {!!plan.redemption_history?.results.length && <details><summary>已有明确结果</summary><RedemptionResults results={plan.redemption_history.results}/></details>}
        </section>}
        <ol className="group-plan">{plan.items.filter(item => !standalone || item.enabled).map(item => <li key={item.app_id}><span>{item.name}</span><Badge appearance="outline">{item.decision === 'no_work' ? (item.app_id === 'charge_plan' ? '无启用体力计划' : '没有待执行内容') : item.decision === 'already_done' ? (item.app_id === 'intel_board' ? '本周目标已完成，跳过' : '本游戏日已完成，跳过') : item.app_id === 'redemption_code' && item.decision === 'run' ? '有待兑换的有效码' : item.app_id === 'intel_board' && item.decision === 'run' ? '本周目标待完成' : item.decision === 'run' && item.completed_run_id ? '本游戏日已完成，本次重跑' : item.enabled ? availability[item.availability] : appIds === undefined ? '未启用' : '本次未选择'}</Badge></li>)}</ol>
        {plan.warnings.map((message, index) => <MessageBar key={index} intent="warning"><MessageBarBody>{message}</MessageBarBody></MessageBar>)}
        {plan.all_done && <MessageBar intent="info"><MessageBarBody>{plan.items.some(item => item.decision === 'no_work') ? '本次没有待执行内容。体力计划及逐项记录已按保存设置筛选。' : plan.items.some(item => item.app_id === 'intel_board' && item.enabled) ? '本次应用已按每日与每周记录完成，无需运行。' : appIds === undefined ? '本游戏日的启用应用均已完成，无需运行。' : '本游戏日的本次所选应用均已完成，无需运行。'}</MessageBarBody></MessageBar>}
        {plan.blockers.map((message, index) => <p className="group-blocker" key={index}>{message}</p>)}
        <Checkbox label={`我确认游戏登录账号为“${plan.context.instance_name}”，并允许本次前台输入${plan.items.some(item => item.app_id === 'drive_disc_dismantle' && item.decision === 'run') ? '；按预览规则拆解驱动盘' : ''}${appCombat ? '；应用内自动战斗将采用上述输入设置' : ''}${standalone && standaloneAppId !== 'world_patrol' ? '；本游戏日已完成的应用也会执行' : ''}${plan.context.rerun_completed ? '；包括本游戏日已完成的应用' : ''}${plan.after_done && plan.after_done !== 'none' ? `；正常完成后${afterDoneName(plan.after_done)}，60 秒内可取消` : ''}`} checked={confirmed} disabled={disabled || active || !plan.executable || !snapshot?.window || !!snapshot.record_error} onChange={(_, data) => setConfirmed(data.checked === true)}/>
        <Button appearance="primary" disabled={disabled || active || !plan.executable || !confirmed || !snapshot?.window || !!snapshot.record_error} onClick={async () => {
          setConfirmed(false); setPlan(null);
          await onCommand(standalone ? { type: 'start_standalone_trial', params: { instance_id: instanceId, group_id: groupId, app_id: standaloneAppId, revision: plan.context.config_revision, game_day: plan.game_day.date, ...(appCombat ? { combat_revision: appCombat.script_revision } : {}), allow_input: true, confirm_account: true } } : { type: 'start_group_trial', params: { instance_id: instanceId, group_id: groupId, revision: plan.context.config_revision, game_day: plan.game_day.date, ...(appCombat ? { combat_revision: appCombat.script_revision } : {}), allow_input: true, confirm_account: true, ...(appIds === undefined ? {} : { app_ids: plan.context.app_ids }), ...(plan.context.rerun_completed ? { rerun_completed: true } : {}) } });
        }}>{standalone ? '开始独立应用试运行' : '开始应用组试运行'}</Button>
      </div>}
      {showStatus && <RunSaveRecovery error={snapshot?.record_error} disabled={disabled} onCommand={onCommand}/>}
      {showStatus && run && <GroupRunStatus run={run} disabled={disabled} recordError={!!snapshot?.record_error} onCommand={onCommand}/>}
    </Card>
    <Card><div className="card-title"><h2>账号运行记录</h2><Button disabled={!connected || historyBusy} onClick={() => void refreshHistory()}>刷新运行记录</Button></div>
      <p>查看所选账号最近的运行记录。按日策略只跳过本游戏日完整保存并确认输入释放的成功应用。</p>
      {historyBusy && <Spinner size="tiny" label="读取运行记录"/>}
      {historyError && <MessageBar intent="error"><MessageBarBody>{historyError}</MessageBarBody></MessageBar>}
      {!!history?.unreadable && <MessageBar intent="warning"><MessageBarBody>{history.unreadable} 份记录无法读取，已保留原文件，请检查数据目录。</MessageBarBody></MessageBar>}
      {history?.has_more && <p>仅展示最近记录，更多历史保留在数据目录中。</p>}
      {history && !history.items.length && <p>该账号暂无可读取的运行记录。</p>}
      <div className="account-history">{history?.items.map(({ record, interrupted }) => <div className="history-item" key={record.id}>
        <div className="card-title"><strong>{record.context?.standalone ? '独立应用运行' : record.group ? '应用组试运行' : record.app_id === 'email' ? '邮件试运行' : record.app_id}</strong><Badge appearance="outline">{interrupted ? '异常中断' : record.mail?.state === 'timed_out' || record.coffee?.state === 'timed_out' ? '超时' : stateNames[record.state]}</Badge></div>
        <p>{record.context?.instance_name} · {record.context?.group_name ?? record.context?.group_id} · 修订 {record.context?.config_revision} · {new Date(record.started_at).toLocaleString()}</p>
        {record.context?.game_day && <p>游戏日 {record.context.game_day.date}</p>}
        {record.context?.rerun_completed && <p>本次重新运行已完成应用</p>}
        <p>{interrupted ? '上次引擎退出前未保存终态，实际结束时间未知，不会自动继续。' : record.message}</p>
        {record.daily?.filter(app => app.redemption).map(app => <RedemptionStatus key={app.app_id} progress={app.redemption!}/>)}
        {record.daily?.filter(app => app.random_play).map(app => <RandomPlayStatus key={app.app_id} progress={app.random_play!}/>)}
        {record.daily?.filter(app => app.team_check).map(app => <TeamCheckStatus key={app.app_id} progress={app.team_check!}/>)}
        {record.daily?.filter(app => app.intel_board).map(app => <IntelBoardStatus key={app.app_id} progress={app}/>)}
        <RunSteps group={record.group}/>
        <code>{record.id}</code>
      </div>)}</div>
    </Card>
  </>;
}
