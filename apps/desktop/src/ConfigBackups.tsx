import { SuibianTempleSummary } from './SuibianTempleSettings';
import { LostVoidSummary } from './LostVoidSettings';
import { ChargePlanSummary } from './ChargePlanSettings';
import { RedemptionConfigSummary } from './RedemptionSettings';
import { dismantleSummary } from './DismantleSettings';
import { randomPlaySummary } from './RandomPlaySettings';
import { intelBoardSummary } from './IntelBoardSettings';
import { TeamSummary } from './TeamSettings';
import { useEffect, useRef, useState } from 'react';
import { combatSummary } from './combatSettings';
import { Badge, Button, Card, Checkbox, Field, MessageBar, MessageBarBody, Select, Spinner } from '@fluentui/react-components';
import type { AccountConfig, AppDefinition, AppGroup, ConfigBackups as BackupList, ConfigBackupPreview } from './generated/protocol';
import { gameRegionOptions, runPolicyOptions } from './generated/protocol';
import { errorMessage, request } from './bridge';
import { captureNames } from './captureSettings';
import { coffeeSummary } from './CoffeeSettings';
import { dailySummary } from './DailySettings';
import { afterDoneName } from './AfterDoneBanner';
import { downloadProxySummary } from './DownloadProxySettings';
import { gameLaunchSummary } from './GameLaunch';
import { runNotificationName } from './RunNotifications';

const label = (group: AppGroup) => group.name ?? (group.id === 'one_dragon' ? '一条龙' : group.id);
const same = (before: unknown, after: unknown) => JSON.stringify(before) === JSON.stringify(after);
const changeName = (before: unknown, after: unknown) => before === undefined ? '将新增' : after === undefined ? '将移除' : same(before, after) ? '未变化' : '有变化';
const union = (before: string[], after: string[]) => [...new Set([...before, ...after])];

function BackupComparison({ preview, apps }: { preview: ConfigBackupPreview; apps: AppDefinition[] }) {
  const { current, backup } = preview;
  const ids = [...new Set([...current.instances.map(item => item.id), ...backup.instances.map(item => item.id)])];
  const [accountId, setAccountId] = useState(() => ids.find(id => !same(current.instances.find(item => item.id === id), backup.instances.find(item => item.id === id))) ?? backup.active_instance);
  const before = current.instances.find(item => item.id === accountId);
  const after = backup.instances.find(item => item.id === accountId);
  const groups = union(before?.groups.map(item => item.id) ?? [], after?.groups.map(item => item.id) ?? []);
  const [groupId, setGroupId] = useState(groups[0] ?? '');
  useEffect(() => { setGroupId(groups.find(id => !same(before?.groups.find(item => item.id === id), after?.groups.find(item => item.id === id))) ?? groups[0] ?? ''); }, [accountId]);
  const oldGroup = before?.groups.find(item => item.id === groupId);
  const newGroup = after?.groups.find(item => item.id === groupId);
  const region = (value: typeof before) => value ? gameRegionOptions.find(option => option.value === value.game_region)?.label : '无此账号';
  const policy = (value: AppGroup | undefined) => value ? runPolicyOptions.find(option => option.value === value.run_policy)?.label : '无此组';
  const keys = (value: AccountConfig) => `暂停 / 继续 ${value.general.shortcuts.pause_resume?.toUpperCase() ?? '关闭'}，停止 ${value.general.shortcuts.stop?.toUpperCase() ?? '关闭'}`;
  return <section aria-label="配置恢复差异" className="backup-comparison">
    <p>当前修订 {current.revision} → 恢复后修订 {current.revision + 1}；备份修订 {backup.revision}。</p>
    <p>当前账号：{current.instances.find(item => item.id === current.active_instance)?.name} → {backup.instances.find(item => item.id === backup.active_instance)?.name}。</p>
    <p>账号：新增 {ids.filter(id => !current.instances.some(item => item.id === id)).length} 个，移除 {ids.filter(id => !backup.instances.some(item => item.id === id)).length} 个，修改 {ids.filter(id => current.instances.some(item => item.id === id) && backup.instances.some(item => item.id === id) && !same(current.instances.find(item => item.id === id), backup.instances.find(item => item.id === id))).length} 个。</p>
    <p>当前账号顺序：{current.instances.map(item => item.name).join(' → ')}；恢复后：{backup.instances.map(item => item.name).join(' → ')}。</p>
    <p>默认截图：{captureNames[current.general.capture_method]} → {captureNames[backup.general.capture_method]}。</p>
    <p>邮件超时：{current.general.mail.timeout_ms} → {backup.general.mail.timeout_ms} ms；启动 / 继续延时：{current.general.mail.start_delay_ms} → {backup.general.mail.start_delay_ms} ms。</p>
    <p>快捷键：{keys(current)} → {keys(backup)}。</p>
    <p>结束后动作：{afterDoneName(current.general.after_done)} → {afterDoneName(backup.general.after_done)}。</p>
    <p>下载代理：{downloadProxySummary(current.general.download_proxy)} → {downloadProxySummary(backup.general.download_proxy)}。</p>
    <div aria-label="兑换码恢复差异"><p>当前配置</p><RedemptionConfigSummary settings={current.redemption}/><p>备份配置</p><RedemptionConfigSummary settings={backup.redemption}/></div>
    <p>旧版已处理码：{before?.redemption_used?.join('、') || '无'} → {after?.redemption_used?.join('、') || '无'}。</p>
    <Field label="核对账号"><Select value={accountId} onChange={(_, data) => setAccountId(Number(data.value))}>{ids.map(id => {
      const old = current.instances.find(item => item.id === id); const next = backup.instances.find(item => item.id === id);
      return <option key={id} value={id}>{next?.name ?? old?.name}（编号 {id} · {changeName(old, next)}）</option>;
    })}</Select></Field>
    <p>账号名称：{before?.name ?? '无此账号'} → {after?.name ?? '无此账号'}；区服：{region(before)} → {region(after)}；参与一条龙：{before ? before.active_in_od ? '是' : '否' : '无此账号'} → {after ? after.active_in_od ? '是' : '否' : '无此账号'}。</p>
    <p className="path-value">游戏启动：{before ? gameLaunchSummary(before.game_launch) : '无此账号'} → {after ? gameLaunchSummary(after.game_launch) : '无此账号'}。</p>
    <p>运行结果提醒：{before ? runNotificationName(before.run_notifications) : '无此账号'} → {after ? runNotificationName(after.run_notifications) : '无此账号'}。</p>
    <p>战斗设置：{before ? combatSummary(before.combat) : '无此账号'} → {after ? combatSummary(after.combat) : '无此账号'}。</p>
    <div aria-label="预备编队恢复差异">{before && <><p>当前编队</p><TeamSummary settings={before.predefined_teams}/></>}{after && <><p>恢复后编队</p><TeamSummary settings={after.predefined_teams}/></>}</div>
    <p>当前应用组顺序：{before?.groups.map(label).join(' → ') ?? '无此账号'}；恢复后：{after?.groups.map(label).join(' → ') ?? '无此账号'}。</p>
    <Field label="核对应用组"><Select value={groupId} onChange={(_, data) => setGroupId(data.value)}>{groups.map(id => {
      const old = before?.groups.find(item => item.id === id); const next = after?.groups.find(item => item.id === id);
      return <option key={id} value={id}>{label((next ?? old)!)}（{changeName(old, next)}）</option>;
    })}</Select></Field>
    <p>组名：{oldGroup ? label(oldGroup) : '无此组'} → {newGroup ? label(newGroup) : '无此组'}；运行策略：{policy(oldGroup)} → {policy(newGroup)}。</p>
    <p>咖啡设置：{oldGroup ? coffeeSummary(oldGroup.coffee) : '无此组'} → {newGroup ? coffeeSummary(newGroup.coffee) : '无此组'}。</p>
    <p>日常设置：{oldGroup ? dailySummary(oldGroup.daily) : '无此组'} → {newGroup ? dailySummary(newGroup.daily) : '无此组'}。</p>
    <p>拆解设置：{oldGroup ? dismantleSummary(oldGroup.dismantle) : '无此组'} → {newGroup ? dismantleSummary(newGroup.dismantle) : '无此组'}。</p>
    <p>录像店设置：{oldGroup ? randomPlaySummary(oldGroup.random_play) : '无此组'} → {newGroup ? randomPlaySummary(newGroup.random_play) : '无此组'}。</p>
    <p>情报板设置：{oldGroup ? intelBoardSummary(oldGroup.intel_board) : '无此组'} → {newGroup ? intelBoardSummary(newGroup.intel_board) : '无此组'}。</p>
    <div aria-label="体力计划恢复差异">{oldGroup && <><p>当前体力计划</p><ChargePlanSummary settings={oldGroup.charge_plan}/></>}{newGroup && <><p>恢复后体力计划</p><ChargePlanSummary settings={newGroup.charge_plan}/></>}</div>
    <div aria-label="随便观恢复差异">{oldGroup && <><p>当前配置</p><SuibianTempleSummary settings={oldGroup.suibian_temple}/></>}{newGroup && <><p>恢复后配置</p><SuibianTempleSummary settings={newGroup.suibian_temple}/></>}</div>
    <div aria-label="迷失之地恢复差异">{oldGroup && <><p>当前配置</p><LostVoidSummary settings={oldGroup.lost_void}/></>}{newGroup && <><p>恢复后配置</p><LostVoidSummary settings={newGroup.lost_void}/></>}</div>
    <div className="input-fields">{[{ group: oldGroup, title: '当前应用顺序' }, { group: newGroup, title: '恢复后应用顺序' }].map(({ group, title }) => {
      return <div role="group" aria-label={title} key={title}><h3>{title}</h3>{group ? <ol className="group-plan">{group.apps.map(item => <li key={item.app_id}>{apps.find(app => app.id === item.app_id)?.name ?? item.app_id} · {item.enabled ? '启用' : '禁用'}</li>)}</ol> : <p>无此组</p>}</div>;
    })}</div>
  </section>;
}

export function ConfigBackups({ connected, active, blocked, current, apps, onLoaded, onBusy }: {
  connected: boolean; active: boolean; blocked: boolean; current: AccountConfig | null; apps: AppDefinition[];
  onLoaded: (config: AccountConfig) => void; onBusy: (busy: boolean) => void;
}) {
  const [list, setList] = useState<BackupList | null>(null);
  const [selected, setSelected] = useState('');
  const [preview, setPreview] = useState<ConfigBackupPreview | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  const key = `${connected}/${active}/${blocked}/${current?.revision}`;
  const currentKey = useRef(key); currentKey.current = key;
  const working = useRef(false);
  const generation = useRef(0);
  const disabled = !connected || active || blocked || busy || !current;

  useEffect(() => {
    generation.current++; working.current = false;
    setPreview(null); setConfirmed(false); setBusy(false); onBusy(false);
    return () => { generation.current++; };
  }, [key, onBusy]);
  useEffect(() => { setList(null); setSelected(''); setError(''); setMessage(''); }, [connected]);

  async function perform(kind: 'list' | 'preview' | 'restore') {
    if (disabled || working.current || (kind === 'restore' && (!preview || !confirmed))) return;
    const requested = key; const requestedGeneration = generation.current;
    const valid = () => currentKey.current === requested && generation.current === requestedGeneration;
    working.current = true; setBusy(true); onBusy(true); setError(''); setMessage(''); setConfirmed(false);
    try {
      const result = await request(kind === 'list' ? { type: 'config_backups' } : kind === 'preview'
        ? { type: 'preview_config_backup', params: { revision: current!.revision, id: selected } }
        : { type: 'restore_config_backup', params: { revision: current!.revision, id: preview!.id, preview_id: preview!.preview_id, confirm: true } });
      if (!valid()) return;
      if (kind === 'list') {
        if (result.type !== 'config_backups') throw new Error('引擎未返回配置备份列表');
        if (result.data.revision !== current!.revision) throw new Error('配置已变化，请先读取最新配置，再刷新备份列表。');
        setList(result.data); setSelected(result.data.items[0]?.id ?? ''); setPreview(null);
      } else if (kind === 'preview') {
        if (result.type !== 'config_backup_preview' || result.data.current.revision !== current!.revision) throw new Error('未取得当前修订的备份预览，请读取最新配置后重试。');
        setPreview(result.data);
      } else {
        if (result.type !== 'account_config') throw new Error('引擎未确认恢复结果，请读取最新配置核对。');
        setPreview(null); setList(null); setSelected(''); onLoaded(result.data);
        setMessage('配置已恢复，恢复前的配置已备份。请核对当前账号及通用设置，再运行任务。');
      }
    } catch (reason) {
      if (!valid()) return;
      setPreview(null);
      if (typeof reason === 'object' && reason && 'code' in reason && reason.code === 'config_conflict') {
        try {
          const latest = await request({ type: 'account_config' });
          if (!valid()) return;
          if (latest.type !== 'account_config') throw new Error('未返回最新配置');
          onLoaded(latest.data); setList(null); setSelected('');
          setError('配置已被修改，已读取最新配置。请刷新备份列表并重新预览。');
        } catch (refreshError) { if (valid()) setError(`配置冲突，读取最新配置失败：${errorMessage(refreshError)}`); }
      } else setError(kind === 'restore' ? `${errorMessage(reason)}。若恢复结果未确认，请读取最新配置核对。` : errorMessage(reason));
    } finally {
      if (valid()) { working.current = false; setBusy(false); onBusy(false); }
    }
  }

  return <Card aria-label="配置备份恢复">
    <div className="card-title"><h2>配置备份与恢复</h2><Badge appearance="outline">先预览再恢复</Badge></div>
    <p>查看保存配置时生成的自动备份，恢复账号、应用组和通用设置。恢复前会再次备份，历史记录和导入原文保留。已移除的账号编号不会分配给新账号。当前配置须能正常读取。</p>
    <Button disabled={disabled} onClick={() => void perform('list')}>读取配置备份</Button>
    {busy && <Spinner size="tiny" label="正在处理配置备份"/>}
    {error && <MessageBar intent="error"><MessageBarBody>{error}</MessageBarBody></MessageBar>}
    {message && <MessageBar intent="success"><MessageBarBody>{message}</MessageBarBody></MessageBar>}
    {list && <>
      <p>显示 {list.items.length} 份备份{list.omitted > 0 ? `，另有 ${list.omitted} 份较早备份保留在数据目录` : ''}。{list.unavailable > 0 ? `${list.unavailable} 个文件不可用或不属于配置备份，未读取内容。` : ''}</p>
      {!list.items.length && <p>暂无配置备份。首次保存后，下一次修改会自动备份之前的配置。</p>}
      {list.items.length > 0 && <><Field label="选择配置备份"><Select value={selected} disabled={disabled} onChange={(_, data) => { setSelected(data.value); setPreview(null); setConfirmed(false); setError(''); setMessage(''); }}>{list.items.map(item => <option key={item.id} value={item.id}>修订 {item.revision} · {item.modified_at ? new Date(item.modified_at).toLocaleString() : '时间未知'} · {item.id}</option>)}</Select></Field>
        <Button disabled={disabled || !selected || list.revision !== current?.revision} onClick={() => void perform('preview')}>预览配置恢复</Button></>}
      {list.revision !== current?.revision && <p>配置已修改，请刷新备份列表后重新预览。</p>}
    </>}
    {preview && <>
      <BackupComparison key={preview.preview_id} preview={preview} apps={apps}/>
      <Checkbox label="我已核对差异，确认恢复账号、应用组和通用设置" checked={confirmed} disabled={disabled} onChange={(_, data) => setConfirmed(data.checked === true)}/>
      <Button appearance="primary" disabled={disabled || !confirmed || preview.current.revision !== current?.revision} onClick={() => void perform('restore')}>确认恢复配置</Button>
    </>}
  </Card>;
}
