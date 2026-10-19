import { useEffect, useRef, useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, MessageBar, MessageBarBody, Select, Spinner } from '@fluentui/react-components';
import type { AccountConfig, AppDefinition, ConfigRecoveryRequest, DesktopConfigRecovery } from './generated/protocol';
import { gameRegionOptions, runPolicyOptions } from './generated/protocol';
import { configRecoveryState, desktop, errorMessage, startConfigRecovery } from './bridge';
import { captureNames } from './captureSettings';
import { coffeeSummary } from './CoffeeSettings';
import { dailySummary } from './DailySettings';
import { gameLaunchSummary } from './GameLaunch';
import { runNotificationName } from './RunNotifications';
import { afterDoneName } from './AfterDoneBanner';
import { downloadProxySummary } from './DownloadProxySettings';

export function useConfigRecovery() {
  const [snapshot, setSnapshot] = useState<DesktopConfigRecovery>({ job: null, catalog: null, preview: null, restored: null });
  const [enabled, setEnabled] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState('');
  const [readError, setReadError] = useState('');
  const mounted = useRef(true);
  const working = useRef(false);
  const serial = useRef(0);
  const applied = useRef(0);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  function accept(value: DesktopConfigRecovery, sequence: number) {
    if (!mounted.current || sequence < applied.current) return;
    applied.current = sequence; setSnapshot(value); setReadError('');
  }
  useEffect(() => {
    if (!enabled || !desktop) return;
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function poll() {
      const sequence = ++serial.current;
      try {
        const value = await configRecoveryState();
        if (alive) accept(value, sequence);
      } catch (reason) { if (alive && sequence >= applied.current) setReadError(`读取离线恢复状态失败：${errorMessage(reason)}`); }
      finally { if (alive) timer = setTimeout(() => void poll(), 500); }
    }
    void poll();
    return () => { alive = false; if (timer) clearTimeout(timer); };
  }, [enabled]);
  async function start(request: ConfigRecoveryRequest) {
    if (!desktop || working.current || snapshot.job?.state === 'running') return;
    working.current = true; setSubmitting(true); setError(''); setEnabled(true);
    const sequence = ++serial.current;
    try {
      const value = await startConfigRecovery(request);
      accept(value, sequence);
      return value;
    } catch (reason) { if (mounted.current) setError(errorMessage(reason)); }
    finally { working.current = false; if (mounted.current) setSubmitting(false); }
  }
  return { snapshot, error, readError, submitting, start };
}

function RecoveryContents({ config, apps }: { config: AccountConfig; apps: AppDefinition[] }) {
  const [accountId, setAccountId] = useState(config.active_instance);
  const account = config.instances.find(item => item.id === accountId)!;
  const [groupId, setGroupId] = useState(account.groups[0].id);
  const group = account.groups.find(item => item.id === groupId) ?? account.groups[0];
  const groupLabel = (value: typeof group) => value.name ?? (value.id === 'one_dragon' ? '一条龙' : value.id);
  return <section aria-label="离线恢复内容" className="backup-comparison">
    <p>恢复后修订 {config.revision}；当前账号：{config.instances.find(item => item.id === config.active_instance)?.name}。</p>
    <p>账号顺序：{config.instances.map(item => item.name).join(' → ')}。</p>
    <p>默认截图：{captureNames[config.general.capture_method]}；邮件超时 {config.general.mail.timeout_ms} ms，启动 / 继续延时 {config.general.mail.start_delay_ms} ms。</p>
    <p>快捷键：暂停 / 继续 {config.general.shortcuts.pause_resume?.toUpperCase() ?? '关闭'}，停止 {config.general.shortcuts.stop?.toUpperCase() ?? '关闭'}。</p>
    <p>结束后动作：{afterDoneName(config.general.after_done)}；正常完成后有 60 秒可取消倒计时。</p>
    <p>下载代理：{downloadProxySummary(config.general.download_proxy)}。</p>
    <Field label="核对恢复账号"><Select value={accountId} onChange={(_, data) => { setAccountId(Number(data.value)); setGroupId(''); }}>{config.instances.map(item => <option key={item.id} value={item.id}>{item.name}（编号 {item.id}）</option>)}</Select></Field>
    <p>区服：{gameRegionOptions.find(item => item.value === account.game_region)?.label}；参与一条龙：{account.active_in_od ? '是' : '否'}。</p>
    <p className="path-value">游戏启动：{gameLaunchSummary(account.game_launch)}。</p>
    <p>运行结果提醒：{runNotificationName(account.run_notifications)}。</p>
    <p>应用组顺序：{account.groups.map(groupLabel).join(' → ')}。</p>
    <Field label="核对恢复应用组"><Select value={group.id} onChange={(_, data) => setGroupId(data.value)}>{account.groups.map(item => <option key={item.id} value={item.id}>{groupLabel(item)}</option>)}</Select></Field>
    <p>运行策略：{runPolicyOptions.find(item => item.value === group.run_policy)?.label}。</p>
    <p>咖啡设置：{coffeeSummary(group.coffee)}。</p>
    <p>日常设置：{dailySummary(group.daily)}。</p>
    <ol aria-label="恢复后应用顺序" className="group-plan">{group.apps.map(item => <li key={item.app_id}>{apps.find(app => app.id === item.app_id)?.name ?? item.app_id} · {item.enabled ? '启用' : '禁用'}</li>)}</ol>
    <details><summary>保留的账号编号（{config.reserved_instance_ids?.length ?? 0} 个）</summary><p className="resource-path">{config.reserved_instance_ids?.join('、') || '无'}</p></details>
  </section>;
}

const states = { damaged: '配置损坏', missing: '配置文件缺失', healthy: '配置可正常读取', unsupported: '配置版本不支持' };
const phases: Record<string, string> = { waiting_engine: '正在核对旧引擎退出与输入收尾', processing_files: '正在处理配置文件', starting_engine: '配置已恢复，正在重连普通权限引擎' };

export function OfflineConfigRecovery({ manager, connected, blocked, apps }: {
  manager: ReturnType<typeof useConfigRecovery>; connected: boolean; blocked: boolean; apps: AppDefinition[];
}) {
  const { snapshot, submitting, error, readError, start } = manager;
  const { catalog, job, restored } = snapshot;
  const [selected, setSelected] = useState('');
  const [previewJob, setPreviewJob] = useState<string | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const mounted = useRef(true);
  const generation = useRef(0);
  const selectable = !connected && !blocked;
  const enabledRef = useRef(selectable); enabledRef.current = selectable;
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; generation.current++; }; }, []);
  useEffect(() => { setConfirmed(false); setPreviewJob(null); generation.current++; }, [connected, blocked]);
  useEffect(() => { if (readError) setConfirmed(false); }, [readError]);
  const running = job?.state === 'running';
  const disabled = !desktop || !selectable || submitting || running;
  const id = selected || catalog?.backups[0]?.id || '';
  const preview = snapshot.preview && previewJob === job?.id && snapshot.preview.id === id && job.state === 'succeeded' ? snapshot.preview : null;
  async function perform(kind: 'list' | 'preview' | 'restore') {
    if (disabled || (kind === 'restore' && (!preview || !confirmed || readError))) return;
    const attempt = ++generation.current;
    const request: ConfigRecoveryRequest = kind === 'list' ? { type: 'list' } : kind === 'preview' ? { type: 'preview', id } : { type: 'restore', id: preview!.id, preview_id: preview!.preview_id, confirm: true };
    setConfirmed(false); setPreviewJob(null);
    if (kind === 'list') setSelected('');
    const value = await start(request);
    if (mounted.current && enabledRef.current && generation.current === attempt && kind === 'preview' && value?.job) setPreviewJob(value.job.id);
  }
  return <Card aria-label="离线配置恢复">
    <div className="card-title"><h2>离线配置恢复</h2><Badge appearance="outline">启动失败时使用</Badge></div>
    <p>配置损坏或丢失时，从当前数据目录的自动备份找回账号、应用组和通用设置。先读取和核对备份，再确认恢复；损坏原文与运行历史保留。</p>
    <Button disabled={disabled} onClick={() => void perform('list')}>读取离线配置备份</Button>
    {connected && <p>引擎已连接，常规恢复请前往“账号与应用组”的配置备份入口。</p>}
    {error && <MessageBar intent="error"><MessageBarBody>{error}</MessageBarBody></MessageBar>}
    {readError && <MessageBar intent="error"><MessageBarBody>{readError}</MessageBarBody></MessageBar>}
    {job?.error && <MessageBar intent="error"><MessageBarBody>{job.error}。请重新读取或预览后重试。</MessageBarBody></MessageBar>}
    {running && <Spinner size="tiny" label={phases[job.phase] ?? '正在恢复配置'}/>}
    {catalog && <>
      <p className="resource-path">数据目录：{catalog.data_directory}</p>
      <p>{states[catalog.current.state]}；显示 {catalog.backups.length} 份备份，另有 {catalog.omitted} 份较早备份、{catalog.unavailable} 个不可用文件。</p>
      {catalog.current.state === 'healthy' && <p>当前配置可正常读取，请重连引擎后使用账号与应用组中的配置备份恢复。</p>}
      {catalog.current.state === 'unsupported' && <p>请使用与配置配套的程序版本，本入口不能覆盖不支持的版本。</p>}
      {!catalog.backups.length && <p>没有可选备份，无法通过本入口恢复配置。</p>}
      {catalog.backups.length > 0 && ['damaged', 'missing'].includes(catalog.current.state) && <>
        <Field label="选择离线配置备份"><Select value={id} disabled={disabled} onChange={(_, data) => { setSelected(data.value); setConfirmed(false); setPreviewJob(null); generation.current++; }}>{catalog.backups.map(item => <option key={item.id} value={item.id}>修订 {item.revision} · {item.modified_at ? new Date(item.modified_at).toLocaleString() : '时间未知'} · {item.id}</option>)}</Select></Field>
        <Button disabled={disabled || !id} onClick={() => void perform('preview')}>预览离线配置恢复</Button>
      </>}
    </>}
    {preview && <>
      <RecoveryContents key={preview.preview_id} config={preview.recovered} apps={apps}/>
      <p>所选备份将作为恢复依据。请核对账号与实际游戏登录是否对应；无法完整比较损坏文件。</p>
      {preview.unreadable_backups > 0 && <p>{preview.unreadable_backups} 份其他备份无法完整读取或校验；所选备份已通过校验。</p>}
      <Checkbox label="我已核对恢复内容，确认恢复配置并重连引擎" disabled={disabled || !!readError} checked={confirmed} onChange={(_, data) => setConfirmed(data.checked === true)}/>
      <Button appearance="primary" disabled={disabled || !confirmed || !!readError} onClick={() => void perform('restore')}>恢复配置并重连引擎</Button>
    </>}
    {restored && <>
      <MessageBar intent="success"><MessageBarBody>配置已恢复为修订 {restored.config.revision}。{job?.reconnected ? '普通权限引擎已重连，请核对账号和设置后手动连接游戏。' : '恢复结果已保存，请确认引擎连接状态。'}</MessageBarBody></MessageBar>
      {restored.preserved_file && <p className="resource-path">损坏原文保留：{restored.preserved_file}</p>}
      <p className="resource-path">恢复结果备份：{restored.recovered_backup}</p>
      {job?.reconnect_error && <MessageBar intent="warning"><MessageBarBody>{job.reconnect_error}</MessageBarBody></MessageBar>}
    </>}
    <p>恢复不会运行游戏任务。切换页面后工作继续，退出桌面会等待文件收尾。</p>
  </Card>;
}
