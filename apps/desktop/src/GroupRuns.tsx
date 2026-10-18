import { useEffect, useRef, useState } from 'react';
import { Badge, Button, Card, Checkbox, MessageBar, MessageBarBody, Spinner } from '@fluentui/react-components';
import type { Command, GroupPlan, RunHistory, Snapshot } from './generated/protocol';
import { gameRegionOptions } from './generated/protocol';
import { errorMessage, request } from './bridge';
import { coffeeSummary } from './CoffeeSettings';
import { GroupRunStatus, isRunActive, RunSaveRecovery, RunSteps, runStateNames as stateNames } from './GroupRunStatus';

const availability = { mail_trial: '邮件试运行', coffee_trial: '咖啡试运行（待实机验收）', unmigrated: '尚无执行器', unknown: '未注册' };

export function GroupRuns({ instanceId, groupId, revision, snapshot, connected, blocked, onCommand, showStatus = true }: {
  instanceId: number; groupId: string; revision: number; snapshot: Snapshot | null;
  connected: boolean; blocked: boolean; onCommand: (command: Command) => Promise<void>; showStatus?: boolean;
}) {
  const [plan, setPlan] = useState<GroupPlan | null>(null);
  const [history, setHistory] = useState<RunHistory | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [planning, setPlanning] = useState(false);
  const [historyBusy, setHistoryBusy] = useState(false);
  const [planError, setPlanError] = useState('');
  const [historyError, setHistoryError] = useState('');
  const currentRun = snapshot?.run;
  const active = isRunActive(currentRun);
  const run = currentRun?.context?.instance_id === instanceId && currentRun.context.group_id === groupId ? currentRun : null;
  const key = `${instanceId}/${groupId}/${revision}/${connected}/${snapshot?.window?.hwnd}/${snapshot?.window?.pid}/${snapshot?.capture_method}/${currentRun?.id}/${currentRun?.state}/${currentRun?.finished_at}`;
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
      const value = await request({ type: 'preview_group', params: { instance_id: instanceId, group_id: groupId } });
      if (currentKey.current !== requested || planRequest.current !== requestId) return;
      if (value.type !== 'group_plan') throw new Error('引擎未返回应用组预览');
      if (value.data.context.config_revision !== revision) throw new Error('配置已更新，请先读取最新账号配置后重新预览。');
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

  const disabled = !connected || blocked || planning;
  return <>
    <Card className="group-run-card"><div className="card-title"><h2>应用组试运行</h2><Badge appearance="outline">邮件与咖啡</Badge></div>
      <p>按保存顺序执行启用项。邮件参数在通用设置中保存，咖啡参数按账号和组保存；咖啡从大世界自动传送并点单，完整流程待实机验收。请确认当天仍可喝咖啡。</p>
      <Button disabled={disabled} onClick={() => void preview()}>预览运行计划</Button>
      {planning && <Spinner size="tiny" label="检查应用组"/>}
      {planError && <MessageBar intent="error"><MessageBarBody>{planError}</MessageBarBody></MessageBar>}
      {plan && <div aria-label="应用组运行计划">
        <p>{plan.context.instance_name} · {plan.context.group_name ?? plan.context.group_id} · 修订 {plan.context.config_revision}</p>
        <p>{gameRegionOptions.find(option => option.value === plan.game_day.region)?.label} · 游戏日 {plan.game_day.date} · {plan.run_policy === 'always' ? '每次运行' : '跳过本游戏日已完成'}</p>
        <p>启动 / 继续延时 {plan.mail_config.start_delay_ms} ms · 邮件超时 {plan.mail_config.timeout_ms} ms</p>
        {plan.items.some(item => item.app_id === 'coffee' && item.enabled) && <p>咖啡：{coffeeSummary(plan.coffee_config)} · 澄辉坪 → 汀曼咖啡</p>}
        <ol className="group-plan">{plan.items.map(item => <li key={item.app_id}><span>{item.name}</span><Badge appearance="outline">{item.decision === 'already_done' ? '本游戏日已完成，跳过' : item.enabled ? availability[item.availability] : '未启用'}</Badge></li>)}</ol>
        {plan.warnings.map((message, index) => <MessageBar key={index} intent="warning"><MessageBarBody>{message}</MessageBarBody></MessageBar>)}
        {plan.all_done && <MessageBar intent="info"><MessageBarBody>本游戏日的启用应用均已完成，无需运行。</MessageBarBody></MessageBar>}
        {plan.blockers.map((message, index) => <p className="group-blocker" key={index}>{message}</p>)}
        <Checkbox label={`我确认游戏登录账号为“${plan.context.instance_name}”，并允许本次前台输入`} checked={confirmed} disabled={disabled || active || !plan.executable || !snapshot?.window || !!snapshot.record_error} onChange={(_, data) => setConfirmed(data.checked === true)}/>
        <Button appearance="primary" disabled={disabled || active || !plan.executable || !confirmed || !snapshot?.window || !!snapshot.record_error} onClick={async () => {
          setConfirmed(false); setPlan(null);
          await onCommand({ type: 'start_group_trial', params: { instance_id: instanceId, group_id: groupId, revision: plan.context.config_revision, game_day: plan.game_day.date, allow_input: true, confirm_account: true } });
        }}>开始应用组试运行</Button>
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
        <div className="card-title"><strong>{record.group ? '应用组试运行' : record.app_id === 'email' ? '邮件试运行' : record.app_id}</strong><Badge appearance="outline">{interrupted ? '异常中断' : record.mail?.state === 'timed_out' || record.coffee?.state === 'timed_out' ? '超时' : stateNames[record.state]}</Badge></div>
        <p>{record.context?.instance_name} · {record.context?.group_name ?? record.context?.group_id} · 修订 {record.context?.config_revision} · {new Date(record.started_at).toLocaleString()}</p>
        {record.context?.game_day && <p>游戏日 {record.context.game_day.date}</p>}
        <p>{interrupted ? '上次引擎退出前未保存终态，实际结束时间未知，不会自动继续。' : record.message}</p>
        <RunSteps group={record.group}/>
        <code>{record.id}</code>
      </div>)}</div>
    </Card>
  </>;
}
