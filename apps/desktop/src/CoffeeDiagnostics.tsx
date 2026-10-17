import { useEffect, useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, Input, MessageBar, MessageBarBody, Select } from '@fluentui/react-components';
import type { CoffeeKind, CoffeePhase, Command, Snapshot } from './generated/protocol';

const phases: Record<CoffeePhase, string> = { inspect: '检查咖啡选项', order: '点单', wait_energy: '等待喝完与电量', confirm_energy: '确认电量', after_energy: '检查喝后画面', confirm_buff: '确认咖啡增益', done: '完成' };
const states: Record<string, string> = { running: '运行中', pausing: '暂停中', paused: '已暂停', stopping: '停止中', succeeded: '完成', failed: '失败', stopped: '已停止' };
export function CoffeeDiagnostics({ snapshot, connected, busy, onCommand }: {
  snapshot: Snapshot | null; connected: boolean; busy: boolean; onCommand: (command: Command) => Promise<void>;
}) {
  const [kind, setKind] = useState<CoffeeKind>('tinman');
  const [timeout, setTimeout] = useState('60000');
  const [delay, setDelay] = useState('5000');
  const [confirmed, setConfirmed] = useState(false);
  const active = !!snapshot?.run && ['running', 'pausing', 'paused', 'stopping'].includes(snapshot.run.state);
  const run = snapshot?.run?.app_id === 'prototype.coffee' ? snapshot.run : null;
  const progress = run?.coffee;
  const integer = (value: string, min: number, max: number) => value.trim() !== '' && Number.isSafeInteger(Number(value)) && Number(value) >= min && Number(value) <= max;
  const valid = integer(timeout, 1, 60000) && integer(delay, 0, 10000);
  const disabled = !connected || busy || active || !!snapshot?.record_error;
  useEffect(() => { setConfirmed(false); }, [connected, snapshot?.window?.hwnd, snapshot?.window?.pid, snapshot?.capture_method, snapshot?.run?.id, snapshot?.record_error]);
  const name = kind === 'tinman' ? '汀曼特调' : '浓缩咖啡';
  return <Card className="coffee-diagnostics">
    <div className="card-title"><h2>咖啡点单试运行</h2><Badge appearance="outline">实机待验收</Badge></div>
    <p>先传送到澄辉坪的汀曼咖啡，手动交互到右侧出现咖啡选项，再启动。流程会选择本次咖啡、等待动画、确认电量和增益，最后返回大世界。</p>
    <MessageBar intent="info"><MessageBarBody>请在延时内切回游戏并保持前台。当前提供汀曼特调与浓缩咖啡的独立试运行，结果保存为诊断记录。</MessageBarBody></MessageBar>
    <div className="input-fields">
      <Field label="本次咖啡"><Select value={kind} disabled={disabled} onChange={(_, data) => { setKind(data.value as CoffeeKind); setConfirmed(false); }}><option value="tinman">汀曼特调</option><option value="espresso">浓缩咖啡</option></Select></Field>
      <Field label="咖啡超时（ms）"><Input type="number" min={1} max={60000} value={timeout} disabled={disabled} onChange={(_, data) => { setTimeout(data.value); setConfirmed(false); }}/></Field>
      <Field label="咖啡启动 / 继续延时（ms）"><Input type="number" min={0} max={10000} value={delay} disabled={disabled} onChange={(_, data) => { setDelay(data.value); setConfirmed(false); }}/></Field>
    </div>
    <Checkbox label={`我确认本次在澄辉坪点单${name}`} checked={confirmed} disabled={disabled || !valid} onChange={(_, data) => setConfirmed(data.checked === true)}/>
    <div className="button-row">
      <Button appearance="primary" disabled={disabled || !snapshot?.window || !valid || !confirmed} onClick={async () => { setConfirmed(false); await onCommand({ type: 'start_coffee', params: { config: { kind, timeout_ms: Number(timeout), start_delay_ms: Number(delay) }, allow_input: true } }); }}>开始咖啡点单</Button>
      <Button disabled={!connected || busy || run?.state !== 'running'} onClick={() => run && onCommand({ type: 'pause', params: { run_id: run.id } })}>暂停咖啡</Button>
      <Button disabled={!connected || busy || run?.state !== 'paused'} onClick={() => run && onCommand({ type: 'resume', params: { run_id: run.id } })}>继续咖啡</Button>
      <Button disabled={!connected || busy || !run || !active || run.state === 'stopping'} onClick={() => run && onCommand({ type: 'stop', params: { run_id: run.id } })}>停止咖啡</Button>
    </div>
    {snapshot?.record_error && <><MessageBar intent="error"><MessageBarBody>{snapshot.record_error}</MessageBarBody></MessageBar><Button disabled={!connected || busy || active} onClick={() => onCommand({ type: 'retry_run_save' })}>重试保存咖啡记录</Button></>}
    {run && <div aria-label="咖啡运行结果"><Badge appearance="tint">{progress?.state === 'timed_out' ? '咖啡超时' : states[run.state]}</Badge><p>{run.message}</p>{progress && <p>步骤：{phases[progress.phase]} · 电量已确认出现：{progress.energy_seen ? '是' : '否'} · {progress.inputs_released ? '输入已释放' : '等待输入收尾'}</p>}<p>运行 ID：{run.id}</p></div>}
  </Card>;
}
