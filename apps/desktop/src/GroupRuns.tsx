import { useEffect, useRef, useState } from 'react';
import { Badge, Button, Card, Checkbox, MessageBar, MessageBarBody, Spinner } from '@fluentui/react-components';
import type { Command, GroupPlan, RunHistory, Snapshot } from './generated/protocol';
import { errorMessage, request } from './bridge';

const stateNames: Record<string, string> = { running: '运行中', pausing: '暂停中', paused: '已暂停', stopping: '停止中', succeeded: '完成', failed: '失败', stopped: '已停止' };
const availability = { mail_trial: '邮件试运行', unmigrated: '尚无执行器', unknown: '未注册' };

export function GroupRuns({ instanceId, groupId, revision, snapshot, connected, blocked, onCommand }: {
  instanceId: number; groupId: string; revision: number; snapshot: Snapshot | null;
  connected: boolean; blocked: boolean; onCommand: (command: Command) => Promise<void>;
}) {
  const [plan, setPlan] = useState<GroupPlan | null>(null);
  const [history, setHistory] = useState<RunHistory | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [planning, setPlanning] = useState(false);
  const [historyBusy, setHistoryBusy] = useState(false);
  const [planError, setPlanError] = useState('');
  const [historyError, setHistoryError] = useState('');
  const currentRun = snapshot?.run;
  const active = !!currentRun && ['running', 'pausing', 'paused', 'stopping'].includes(currentRun.state);
  const run = currentRun?.context?.instance_id === instanceId && currentRun.context.group_id === groupId ? currentRun : null;
  const key = `${instanceId}/${groupId}/${revision}/${connected}/${snapshot?.window?.hwnd}/${snapshot?.window?.pid}/${snapshot?.capture_method}`;
  const currentKey = useRef(key); currentKey.current = key;

  useEffect(() => { setPlan(null); setConfirmed(false); setPlanError(''); }, [key]);
  useEffect(() => { setConfirmed(false); }, [snapshot?.record_error, active]);

  async function preview() {
    const requested = key;
    setPlanning(true); setPlanError(''); setConfirmed(false); setPlan(null);
    try {
      const value = await request({ type: 'preview_group', params: { instance_id: instanceId, group_id: groupId } });
      if (currentKey.current !== requested) return;
      if (value.type !== 'group_plan') throw new Error('引擎未返回应用组预览');
      if (value.data.context.config_revision !== revision) throw new Error('配置已更新，请先读取最新账号配置后重新预览。');
      setPlan(value.data);
    } catch (reason) { if (currentKey.current === requested) setPlanError(errorMessage(reason)); }
    finally { setPlanning(false); }
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
    const requested = key;
    try {
      const value = await request({ type: 'run_history', params: { instance_id: instanceId } });
      if (currentKey.current === requested && value.type === 'run_history') setHistory(value.data);
    } catch (reason) { if (currentKey.current === requested) setHistoryError(errorMessage(reason)); }
    finally { setHistoryBusy(false); }
  }

  const disabled = !connected || blocked || planning;
  return <>
    <Card className="group-run-card"><div className="card-title"><h2>应用组试运行</h2><Badge appearance="outline">当前支持邮件</Badge></div>
      <p>先预览已保存的应用组。当前仅支持邮件试运行，所有启用项通过检查后才能开始；启动和继续后有 3 秒时间切回游戏。</p>
      <Button disabled={disabled} onClick={() => void preview()}>预览运行计划</Button>
      {planning && <Spinner size="tiny" label="检查应用组"/>}
      {planError && <MessageBar intent="error"><MessageBarBody>{planError}</MessageBarBody></MessageBar>}
      {plan && <div aria-label="应用组运行计划">
        <p>{plan.context.instance_name} · {plan.context.group_id} · 修订 {plan.context.config_revision}</p>
        <ol className="group-plan">{plan.items.map(item => <li key={item.app_id}><span>{item.name}</span><Badge appearance="outline">{item.enabled ? availability[item.availability] : '未启用'}</Badge></li>)}</ol>
        {plan.blockers.map((message, index) => <p className="group-blocker" key={index}>{message}</p>)}
        <Checkbox label={`我确认游戏登录账号为“${plan.context.instance_name}”，并允许本次前台输入`} checked={confirmed} disabled={disabled || active || !plan.executable || !snapshot?.window || !!snapshot.record_error} onChange={(_, data) => setConfirmed(data.checked === true)}/>
        <Button appearance="primary" disabled={disabled || active || !plan.executable || !confirmed || !snapshot?.window || !!snapshot.record_error} onClick={async () => {
          setConfirmed(false); setPlan(null);
          await onCommand({ type: 'start_group_trial', params: { instance_id: instanceId, group_id: groupId, revision: plan.context.config_revision, config: { timeout_ms: 60000, start_delay_ms: 3000 }, allow_input: true, confirm_account: true } });
        }}>开始应用组试运行</Button>
      </div>}
      {snapshot?.record_error && <><MessageBar intent="error"><MessageBarBody>{snapshot.record_error}。请修复数据目录后重试保存，当前不接受新任务。</MessageBarBody></MessageBar><Button disabled={disabled} onClick={() => onCommand({ type: 'retry_run_save' })}>重试保存运行记录</Button></>}
      {run && <div aria-label="应用组运行状态">
        <Badge appearance="tint">{run.mail?.state === 'timed_out' ? '超时' : stateNames[run.state]}</Badge><p>{run.message}</p>
        <p>{run.mail?.inputs_released ? '输入已释放' : '等待输入收尾'} · 运行 ID：{run.id}</p>
        <div className="button-row">
          <Button disabled={disabled || run.state !== 'running'} onClick={() => onCommand({ type: 'pause', params: { run_id: run.id } })}>暂停应用组</Button>
          <Button disabled={disabled || run.state !== 'paused'} onClick={() => onCommand({ type: 'resume', params: { run_id: run.id } })}>继续应用组</Button>
          <Button disabled={disabled || !active || run.state === 'stopping'} onClick={() => onCommand({ type: 'stop', params: { run_id: run.id } })}>停止应用组</Button>
        </div>
      </div>}
    </Card>
    <Card><div className="card-title"><h2>账号运行记录</h2><Button disabled={!connected || historyBusy} onClick={() => void refreshHistory()}>刷新运行记录</Button></div>
      <p>查看所选账号最近的运行记录。历史结果暂不用于自动跳过每日任务。</p>
      {historyBusy && <Spinner size="tiny" label="读取运行记录"/>}
      {historyError && <MessageBar intent="error"><MessageBarBody>{historyError}</MessageBarBody></MessageBar>}
      {!!history?.unreadable && <MessageBar intent="warning"><MessageBarBody>{history.unreadable} 份记录无法读取，已保留原文件，请检查数据目录。</MessageBarBody></MessageBar>}
      {history?.has_more && <p>仅展示最近记录，更多历史保留在数据目录中。</p>}
      {history && !history.items.length && <p>该账号暂无可读取的运行记录。</p>}
      <div className="account-history">{history?.items.map(({ record, interrupted }) => <div className="history-item" key={record.id}>
        <div className="card-title"><strong>{record.app_id === 'email' ? '邮件试运行' : record.app_id}</strong><Badge appearance="outline">{interrupted ? '异常中断' : record.mail?.state === 'timed_out' ? '超时' : stateNames[record.state]}</Badge></div>
        <p>{record.context?.instance_name} · {record.context?.group_id} · 修订 {record.context?.config_revision} · {new Date(record.started_at).toLocaleString()}</p>
        <p>{interrupted ? '上次引擎退出前未保存终态，实际结束时间未知，不会自动继续。' : record.message}</p>
        <code>{record.id}</code>
      </div>)}</div>
    </Card>
  </>;
}
