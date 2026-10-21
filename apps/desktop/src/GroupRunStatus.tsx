import { RedemptionStatus } from './RedemptionSettings';
import { IntelBoardStatus } from './IntelBoardSettings';
import { RandomPlayStatus } from './RandomPlaySettings';
import { TeamCheckStatus } from './TeamSettings';
import { Badge, Button, MessageBar, MessageBarBody } from '@fluentui/react-components';
import type { Command, GroupProgress, RunRecord } from './generated/protocol';
import { trialAppOptions } from './generated/protocol';
export const trialAppName = (id: string) => trialAppOptions.find(app => app.id === id)?.label ?? id;

export const runStateNames: Record<string, string> = { running: '运行中', pausing: '暂停中', paused: '已暂停', stopping: '停止中', succeeded: '完成', failed: '失败', stopped: '已停止' };
export const isRunActive = (run: RunRecord | null | undefined) => !!run && ['running', 'pausing', 'paused', 'stopping'].includes(run.state);
export const groupInputsReleased = (run: RunRecord) => run.group
  ? run.group.steps.every(step => step.inputs_released) && (run.mail?.inputs_released ?? true) && (run.coffee?.inputs_released ?? true) && (run.daily ?? []).every(item => item.inputs_released)
  : run.combat?.inputs_released ?? run.mail?.inputs_released ?? run.coffee?.inputs_released ?? false;

export function RunSteps({ group }: { group: GroupProgress | undefined }) {
  if (!group) return null;
  const names = { pending: '等待执行', running: '执行中', skipped: '已跳过', succeeded: '执行完成', failed: '执行失败', stopped: '执行停止' };
  return <ol className="group-plan" aria-label="应用分项进度">{group.steps.map(step => <li key={step.app_id}>
    <span>{trialAppName(step.app_id)}</span>
    <Badge appearance="outline">{step.state === 'running' && group.state === 'paused' ? '执行已暂停' : names[step.state]}</Badge>
    {step.state !== 'pending' && step.state !== 'skipped' && <span className="group-step-release">{step.inputs_released ? '分项输入已释放' : '分项输入待释放'}</span>}
  </li>)}</ol>;
}

export function RunSaveRecovery({ error, disabled, onCommand }: { error: string | undefined; disabled: boolean; onCommand: (command: Command) => Promise<void> }) {
  return error ? <><MessageBar intent="error"><MessageBarBody>{error}。请修复数据目录后重试保存，当前不接受新任务。</MessageBarBody></MessageBar><Button disabled={disabled} onClick={() => onCommand({ type: 'retry_run_save' })}>重试保存运行记录</Button></> : null;
}

export function GroupRunStatus({ run, disabled, recordError, onCommand }: {
  run: RunRecord; disabled: boolean; recordError: boolean; onCommand: (command: Command) => Promise<void>;
}) {
  return <div aria-label="应用组运行状态">
    <Badge appearance="tint">{run.mail?.state === 'timed_out' || run.coffee?.state === 'timed_out' || run.daily?.some(item => item.state === 'timed_out') ? '超时' : runStateNames[run.state]}</Badge><p>{run.message}</p>
    {run.daily?.map(item => <p key={item.app_id}>{trialAppName(item.app_id)} · {item.completion_proven ? '已确认完成' : '等待完成确认'}{item.dismantle && <span> · {item.dismantle.obtained ? '已获得拆解材料' : item.dismantle.empty_selection ? '没有符合条件的驱动盘' : item.dismantle.confirmation_attempted ? '已提交拆解，等待结果' : item.dismantle.filter_applied ? '已应用筛选' : '正在准备筛选'}</span>} · 有效耗时 {item.active_ms} ms</p>)}
    <p>{groupInputsReleased(run) ? '输入已释放' : '等待输入收尾'} · 运行 ID：{run.id}</p>
    {run.daily?.filter(item => item.redemption).map(item => <RedemptionStatus key={item.app_id} progress={item.redemption!}/>)}
    {run.daily?.filter(item => item.random_play).map(item => <RandomPlayStatus key={item.app_id} progress={item.random_play!}/>)}
    {run.daily?.filter(item => item.team_check).map(item => <TeamCheckStatus key={item.app_id} progress={item.team_check!}/>)}
    {run.daily?.filter(item => item.intel_board).map(item => <IntelBoardStatus key={item.app_id} progress={item}/>)}
    <RunSteps group={run.group}/>
    <div className="button-row">
      <Button disabled={disabled || run.state !== 'running'} onClick={() => onCommand({ type: 'pause', params: { run_id: run.id } })}>{run.context?.standalone ? '暂停应用' : '暂停应用组'}</Button>
      <Button disabled={disabled || recordError || run.state !== 'paused'} onClick={() => onCommand({ type: 'resume', params: { run_id: run.id } })}>{run.context?.standalone ? '继续应用' : '继续应用组'}</Button>
      <Button disabled={disabled || !isRunActive(run) || run.state === 'stopping'} onClick={() => onCommand({ type: 'stop', params: { run_id: run.id } })}>{run.context?.standalone ? '停止应用' : '停止应用组'}</Button>
    </div>
  </div>;
}
