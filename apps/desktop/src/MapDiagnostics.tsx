import { useEffect, useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, Input, MessageBar, MessageBarBody, Spinner } from '@fluentui/react-components';
import type { Command, MapPhase, Snapshot } from './generated/protocol';

const phases: Record<MapPhase, string> = { inspect: '检查地图起点', select_area: '选择澄辉坪', select_point: '选择汀曼咖啡', inspect_dialog: '等待传送弹窗', confirm: '确认传送', wait_arrival: '等待大世界', done: '完成' };
const states: Record<string, string> = { running: '运行中', pausing: '暂停中', paused: '已暂停', stopping: '停止中', succeeded: '完成', failed: '失败', stopped: '已停止' };
const integer = (value: string, min: number, max: number) => value.trim() !== '' && Number.isSafeInteger(Number(value)) && Number(value) >= min && Number(value) <= max;

export function MapDiagnostics({ snapshot, connected, busy, onCommand }: {
  snapshot: Snapshot | null; connected: boolean; busy: boolean; onCommand: (command: Command) => Promise<void>;
}) {
  const [timeout, setTimeout] = useState('60000');
  const [delay, setDelay] = useState('3000');
  const [confirmed, setConfirmed] = useState(false);
  const active = !!snapshot?.run && ['running', 'pausing', 'paused', 'stopping'].includes(snapshot.run.state);
  const run = snapshot?.run?.app_id === 'prototype.map_transport' ? snapshot.run : null;
  const progress = run?.map;
  const valid = integer(timeout, 1, 60000) && integer(delay, 0, 10000);
  const disabled = !connected || busy || active || !!snapshot?.record_error;
  useEffect(() => { setConfirmed(false); }, [connected, snapshot?.window?.hwnd, snapshot?.window?.pid, snapshot?.capture_method, snapshot?.run?.id, snapshot?.record_error]);
  return <Card className="map-diagnostics">
    <div className="card-title"><h2>地图传送试运行</h2><Badge appearance="outline">实机待验收</Badge></div>
    <p>固定路线：澄辉坪 → 汀曼咖啡。先在游戏按 N 打开普通地图，再启动；请在延时期间切回游戏并保持前台。目标须已解锁且在当前列表可见。</p>
    <MessageBar intent="info"><MessageBarBody>暂停后保留传送检查点，继续时重新识别画面；确认结果不明确时会停止等待或超时。完成后请核对落地位置。本入口不喝咖啡，也不记录每日完成。</MessageBarBody></MessageBar>
    {snapshot?.record_error && <><MessageBar intent="error"><MessageBarBody>{snapshot.record_error}。请修复数据目录后重试保存。</MessageBarBody></MessageBar><Button disabled={!connected || busy || active} onClick={() => onCommand({ type: 'retry_run_save' })}>重试保存运行记录</Button></>}
    <div className="input-fields">
      <Field label="地图超时（ms）"><Input type="number" min={1} max={60000} value={timeout} disabled={disabled} onChange={(_, data) => { setTimeout(data.value); setConfirmed(false); }}/></Field>
      <Field label="地图启动 / 继续延时（ms）"><Input type="number" min={0} max={10000} value={delay} disabled={disabled} onChange={(_, data) => { setDelay(data.value); setConfirmed(false); }}/></Field>
    </div>
    <Checkbox label="我确认本次传送至澄辉坪的汀曼咖啡" checked={confirmed} disabled={disabled || !valid} onChange={(_, data) => setConfirmed(data.checked === true)}/>
    <div className="button-row">
      <Button appearance="primary" disabled={disabled || !snapshot?.window || !valid || !confirmed} onClick={async () => {
        setConfirmed(false); await onCommand({ type: 'start_map', params: { config: { timeout_ms: Number(timeout), start_delay_ms: Number(delay) }, allow_input: true } });
      }}>开始地图传送</Button>
      <Button disabled={!connected || busy || run?.state !== 'running'} onClick={() => run && onCommand({ type: 'pause', params: { run_id: run.id } })}>暂停地图</Button>
      <Button disabled={!connected || busy || run?.state !== 'paused'} onClick={() => run && onCommand({ type: 'resume', params: { run_id: run.id } })}>继续地图</Button>
      <Button disabled={!connected || busy || !run || !active || run.state === 'stopping'} onClick={() => run && onCommand({ type: 'stop', params: { run_id: run.id } })}>停止地图</Button>
    </div>
    {run && <div aria-label="地图运行结果">
      <Badge appearance="tint">{progress?.state === 'timed_out' ? '地图超时' : states[run.state]}</Badge>
      <p>{run.message}</p>
      {progress && <p>步骤：{phases[progress.phase]} · 有效运行 {progress.active_ms} ms · {progress.inputs_released ? '输入已释放' : '等待输入收尾'}</p>}
      <p>运行 ID：{run.id}</p>
      {run.state === 'pausing' && <Spinner size="tiny" label="正在等待地图暂停与输入释放"/>}
      {run.state === 'stopping' && <Spinner size="tiny" label="正在等待地图停止完成"/>}
    </div>}
  </Card>;
}
