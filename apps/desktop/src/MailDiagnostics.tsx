import { useEffect, useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, Input, MessageBar, MessageBarBody, Spinner } from '@fluentui/react-components';
import type { Command, MailPhase, Snapshot } from './generated/protocol';

const phases: Record<MailPhase, string> = { navigate: '进入邮件', inspect: '重新识别画面', claim: '领取奖励', after_claim: '核对领取结果', confirm: '确认奖励弹窗', after_confirm: '核对已领取状态', return_list: '返回菜单', return_world: '返回大世界', done: '完成' };
const states: Record<string, string> = { running: '运行中', pausing: '暂停中', paused: '已暂停', stopping: '停止中', succeeded: '完成', failed: '失败', stopped: '已停止' };
const integer = (value: string, min: number, max: number) => value.trim() !== '' && Number.isSafeInteger(Number(value)) && Number(value) >= min && Number(value) <= max;

export function MailDiagnostics({ snapshot, connected, busy, onCommand }: {
  snapshot: Snapshot | null; connected: boolean; busy: boolean; onCommand: (command: Command) => Promise<void>;
}) {
  const [timeout, setTimeout] = useState('60000');
  const [delay, setDelay] = useState('3000');
  const [confirmed, setConfirmed] = useState(false);
  const active = !!snapshot?.run && ['running', 'pausing', 'paused', 'stopping'].includes(snapshot.run.state);
  const run = snapshot?.run?.app_id === 'email' ? snapshot.run : null;
  const progress = run?.mail;
  const valid = integer(timeout, 1, 60000) && integer(delay, 0, 10000);
  const disabled = !connected || busy || active;
  useEffect(() => { setConfirmed(false); }, [connected, snapshot?.window?.hwnd, snapshot?.window?.pid, snapshot?.capture_method]);
  return <Card className="mail-diagnostics">
    <div className="card-title"><h2>邮件流程试运行</h2><Badge appearance="outline">主流程通过 · 边界待验收</Badge></div>
    <p>从大世界、菜单或邮件列表开始，领取附件后返回大世界。只支持前台键鼠；启动或继续后，请在延时期间切回游戏并保持前台。</p>
    <MessageBar intent="info"><MessageBarBody>暂停会等待输入释放并保留本次领取进度；暂停期间仍占用运行槽。引擎退出后不会自动继续领取。</MessageBarBody></MessageBar>
    <div className="input-fields">
      <Field label="邮件超时（ms）"><Input type="number" min={1} max={60000} value={timeout} disabled={disabled} onChange={(_, data) => { setTimeout(data.value); setConfirmed(false); }}/></Field>
      <Field label="邮件启动 / 继续延时（ms）"><Input type="number" min={0} max={10000} value={delay} disabled={disabled} onChange={(_, data) => { setDelay(data.value); setConfirmed(false); }}/></Field>
    </div>
    <Checkbox label="我确认运行本次邮件领奖流程" checked={confirmed} disabled={disabled || !valid} onChange={(_, data) => setConfirmed(data.checked === true)}/>
    <div className="button-row">
      <Button appearance="primary" disabled={disabled || !snapshot?.window || !valid || !confirmed} onClick={async () => {
        setConfirmed(false); await onCommand({ type: 'start_mail', params: { config: { timeout_ms: Number(timeout), start_delay_ms: Number(delay) }, allow_input: true } });
      }}>开始邮件流程</Button>
      <Button disabled={!connected || busy || run?.state !== 'running'} onClick={() => run && onCommand({ type: 'pause', params: { run_id: run.id } })}>暂停邮件</Button>
      <Button disabled={!connected || busy || run?.state !== 'paused'} onClick={() => run && onCommand({ type: 'resume', params: { run_id: run.id } })}>继续邮件</Button>
      <Button disabled={!connected || busy || !run || !active || run.state === 'stopping'} onClick={() => run && onCommand({ type: 'stop', params: { run_id: run.id } })}>停止邮件</Button>
    </div>
    {run && <div aria-label="邮件运行结果">
      <Badge appearance="tint">{progress?.state === 'timed_out' ? '邮件超时' : states[run.state]}</Badge>
      <p>{run.message}</p>
      {progress && <p>步骤：{phases[progress.phase]} · 有效运行 {progress.active_ms} ms · {progress.inputs_released ? '输入已释放' : '等待输入收尾'}</p>}
      <p>运行 ID：{run.id}</p>
      {run.state === 'pausing' && <Spinner size="tiny" label="正在等待暂停与输入释放"/>}
      {run.state === 'stopping' && <Spinner size="tiny" label="正在等待邮件停止完成"/>}
    </div>}
  </Card>;
}
