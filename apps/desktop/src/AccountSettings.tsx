import { useEffect, useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, Input, MessageBar, MessageBarBody, Select, Spinner } from '@fluentui/react-components';
import type { AccountConfig, AppDefinition, Command, ConfigChange, GameRegion, RunPolicy, Snapshot } from './generated/protocol';
import { gameRegionOptions, runPolicyOptions } from './generated/protocol';
import { errorMessage, request } from './bridge';
import { GroupRuns } from './GroupRuns';
import { ConfigImport } from './ConfigImport';

export function AccountSettings({ connected, active, blocked, apps, snapshot, onCommand, onImported }: { connected: boolean; active: boolean; blocked: boolean; apps: AppDefinition[]; snapshot: Snapshot | null; onCommand: (command: Command) => Promise<void>; onImported: (config: AccountConfig) => void }) {
  const [config, setConfig] = useState<AccountConfig | null>(null);
  const [instanceId, setInstanceId] = useState(0);
  const [groupId, setGroupId] = useState('one_dragon');
  const [name, setName] = useState('');
  const [enabled, setEnabled] = useState(true);
  const [region, setRegion] = useState<GameRegion>('cn');
  const [runPolicy, setRunPolicy] = useState<RunPolicy>('always');
  const [newName, setNewName] = useState('');
  const [busy, setBusy] = useState(false);
  const [importBusy, setImportBusy] = useState(false);
  const [error, setError] = useState('');
  const instance = config?.instances.find(item => item.id === instanceId);
  const group = instance?.groups.find(item => item.id === groupId);
  const disabled = !connected || blocked || busy || importBusy || active;

  function accept(value: AccountConfig) {
    setConfig(value);
    setInstanceId(current => value.instances.some(item => item.id === current) ? current : value.active_instance);
  }

  async function load() {
    const result = await request({ type: 'account_config' });
    if (result.type !== 'account_config') throw new Error('引擎未返回账号配置');
    accept(result.data);
  }

  useEffect(() => {
    let alive = true;
    setConfig(null); setError('');
    if (connected) request({ type: 'account_config' }).then(result => {
      if (alive && result.type === 'account_config') accept(result.data);
    }).catch(reason => { if (alive) setError(errorMessage(reason)); });
    return () => { alive = false; };
  }, [connected]);

  useEffect(() => {
    setName(instance?.name ?? ''); setEnabled(instance?.active_in_od ?? true);
  }, [instance?.id, instance?.name, instance?.active_in_od]);

  useEffect(() => { setRegion(instance?.game_region ?? 'cn'); }, [instance?.id, instance?.game_region]);
  useEffect(() => { setRunPolicy(group?.run_policy ?? 'always'); }, [instance?.id, group?.id, group?.run_policy]);

  useEffect(() => {
    setGroupId(current => instance?.groups.some(item => item.id === current) ? current : instance?.groups[0]?.id ?? 'one_dragon');
  }, [instance?.id, instance?.groups]);

  async function update(change: ConfigChange) {
    if (!config || disabled) return;
    setBusy(true); setError('');
    try {
      const result = await request({ type: 'update_account_config', params: { revision: config.revision, change } });
      if (result.type !== 'account_config') throw new Error('引擎未返回保存结果');
      accept(result.data);
      if (change.type === 'create_instance') setNewName('');
    } catch (reason) {
      if (typeof reason === 'object' && reason && 'code' in reason && reason.code === 'config_conflict') {
        try { await load(); setError('配置已被其他窗口或进程修改，已加载最新配置。请核对后重新编辑。'); }
        catch (refreshError) { setConfig(null); setError(`配置冲突，读取最新配置失败：${errorMessage(refreshError)}`); }
      } else setError(errorMessage(reason));
    } finally { setBusy(false); }
  }

  function move(index: number, offset: number) {
    if (!group || !instance) return;
    const items = [...group.apps];
    [items[index], items[index + offset]] = [items[index + offset], items[index]];
    void update({ type: 'save_group', instance_id: instance.id, group_id: group.id, apps: items });
  }

  return <>
    <div className="page-heading"><div><div className="eyebrow">ACCOUNTS AND APPLICATIONS</div><h1>账号与应用组</h1><p>管理本地账号配置和应用顺序。切换配置后，请自行确认游戏登录账号。</p></div><Button disabled={!connected || busy || importBusy || blocked} onClick={async () => {
      setBusy(true); setError(''); try { await load(); } catch (reason) { setError(errorMessage(reason)); } finally { setBusy(false); }
    }}>读取最新配置</Button></div>
    {error && <MessageBar intent="error"><MessageBarBody>{error}</MessageBarBody></MessageBar>}
    {active && <MessageBar intent="info"><MessageBarBody>任务仍在运行或收尾，请停止任务后再修改账号及应用组。</MessageBarBody></MessageBar>}
    <ConfigImport connected={connected} active={active} blocked={blocked || busy} current={config} apps={apps} onBusy={setImportBusy} onImported={value => { accept(value); setError(''); onImported(value); }}/>
    {!config && <Card><p>{connected ? '正在读取账号配置。读取失败时可点击“读取最新配置”重试。' : '连接桌面引擎后可读取和编辑本地配置。'}</p></Card>}
    {config && <>
      <Card><div className="card-title"><h2>账号配置</h2><Badge appearance="outline">修订 {config.revision}</Badge></div>
        <p>当前账号：{config.instances.find(item => item.id === config.active_instance)?.name}</p>
        <div className="input-fields">
          <Field label="编辑账号"><Select value={instanceId} disabled={busy} onChange={(_, data) => setInstanceId(Number(data.value))}>{config.instances.map(item => <option key={item.id} value={item.id}>{item.name}{item.id === config.active_instance ? '（当前）' : ''}</option>)}</Select></Field>
          <Field label="账号名称"><Input value={name} maxLength={80} disabled={disabled} onChange={(_, data) => setName(data.value)}/></Field>
        </div>
        <Checkbox label="参与一条龙" checked={enabled} disabled={disabled} onChange={(_, data) => setEnabled(data.checked === true)}/>
        <Field label="账号区服"><Select value={region} disabled={disabled} onChange={(_, data) => setRegion(data.value as GameRegion)}>{gameRegionOptions.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}</Select></Field>
        <p>区服用于计算每日重置日期，启用按日跳过前请确认与游戏一致。</p>
        <Button disabled={disabled || !instance || region === instance.game_region} onClick={() => instance && void update({ type: 'set_game_region', instance_id: instance.id, game_region: region })}>保存区服</Button>
        <div className="button-row"><Button disabled={disabled || !instance || !name.trim()} onClick={() => instance && void update({ type: 'update_instance', instance_id: instance.id, name, active_in_od: enabled })}>保存账号</Button>
          <Button appearance="primary" disabled={disabled || !instance || instance.id === config.active_instance} onClick={() => instance && void update({ type: 'switch_instance', instance_id: instance.id })}>设为当前账号</Button>
          {busy && <Spinner size="tiny" label="保存配置"/>}
        </div>
        <div className="account-create"><Field label="新增账号名称"><Input value={newName} maxLength={80} disabled={disabled} onChange={(_, data) => setNewName(data.value)}/></Field><Button disabled={disabled || !newName.trim() || config.instances.length >= 128} onClick={() => void update({ type: 'create_instance', name: newName })}>新增账号</Button></div>
      </Card>
      {instance && <Card><div className="card-title"><h2>应用组顺序</h2><Badge appearance="outline">{group?.apps.length ?? 0} 项</Badge></div>
        <Field label="应用组"><Select value={groupId} disabled={busy} onChange={(_, data) => setGroupId(data.value)}>{instance.groups.map(item => <option key={item.id} value={item.id}>{item.id === 'one_dragon' ? '一条龙' : item.id}</option>)}</Select></Field>
        <Field label="运行策略"><Select value={runPolicy} disabled={disabled} onChange={(_, data) => setRunPolicy(data.value as RunPolicy)}>{runPolicyOptions.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}</Select></Field>
        <Button disabled={disabled || !group || runPolicy === group.run_policy} onClick={() => group && void update({ type: 'set_run_policy', instance_id: instance.id, group_id: group.id, run_policy: runPolicy })}>保存运行策略</Button>
        <p>新账号的应用默认禁用。启用和排序会保存到当前编辑账号；应用完成迁移与验收后才开放运行。</p>
        <div className="account-apps">{group?.apps.map((item, index) => {
          const app = apps.find(app => app.id === item.app_id);
          return <div className="account-app" key={item.app_id} data-app-id={item.app_id}>
            <Checkbox label={app?.name ?? item.app_id} checked={item.enabled} disabled={disabled} onChange={(_, data) => void update({ type: 'save_group', instance_id: instance.id, group_id: group.id, apps: group.apps.map((entry, i) => i === index ? { ...entry, enabled: data.checked === true } : entry) })}/>
            <Badge appearance="outline">{!app ? '未注册' : app.implemented ? '可运行' : '待迁移'}</Badge>
            <Button size="small" disabled={disabled || index === 0} aria-label={`上移 ${app?.name ?? item.app_id}`} onClick={() => move(index, -1)}>上移</Button>
            <Button size="small" disabled={disabled || index === group.apps.length - 1} aria-label={`下移 ${app?.name ?? item.app_id}`} onClick={() => move(index, 1)}>下移</Button>
          </div>;
        })}</div>
      </Card>}
      {instance && group && <GroupRuns instanceId={instance.id} groupId={group.id} revision={config.revision} snapshot={snapshot} connected={connected} blocked={blocked || importBusy || busy} onCommand={onCommand}/>}
    </>}
  </>;
}
