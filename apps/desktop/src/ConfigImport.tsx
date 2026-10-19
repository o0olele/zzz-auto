import { useEffect, useRef, useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, Input, MessageBar, MessageBarBody, Spinner } from '@fluentui/react-components';
import type { AccountConfig, AppDefinition, ImportPreview } from './generated/protocol';
import { gameRegionOptions } from './generated/protocol';
import { errorMessage, pickConfigImportSource, request } from './bridge';
import { captureNames } from './captureSettings';
import { coffeeSummary } from './CoffeeSettings';
import { gameLaunchSummary } from './GameLaunch';
import { runNotificationName } from './RunNotifications';
import { afterDoneName } from './AfterDoneBanner';

export function ConfigImport({ connected, active, blocked, current, apps, onImported, onBusy }: {
  connected: boolean; active: boolean; blocked: boolean; current: AccountConfig | null; apps: AppDefinition[];
  onImported: (config: AccountConfig) => void; onBusy: (busy: boolean) => void;
}) {
  const [source, setSource] = useState('');
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  const generation = useRef(0);
  const working = useRef(false);
  const disabled = !connected || active || blocked || busy || !current;
  const canApply = !!current && !!preview && (current.revision === 0 || current.import_id === preview.preview_id);

  useEffect(() => {
    generation.current++;
    working.current = false;
    setPreview(null); setConfirmed(false); setBusy(false); setError(''); setMessage(''); onBusy(false);
    return () => { generation.current++; };
  }, [connected, active, blocked, onBusy]);

  function changeSource(value: string) {
    setSource(value); setPreview(null); setConfirmed(false); setError(''); setMessage('');
  }

  async function perform(kind: 'pick' | 'preview' | 'apply') {
    if (disabled || working.current || (kind === 'apply' && (!canApply || !confirmed))) return;
    const requested = generation.current;
    working.current = true; setBusy(true); onBusy(true); setError(''); setMessage('');
    try {
      if (kind === 'pick') {
        const value = await pickConfigImportSource();
        if (generation.current === requested && value !== null) changeSource(value);
        return;
      }
      const result = await request(kind === 'preview'
        ? { type: 'preview_config_import', params: { source: source.trim() } }
        : { type: 'apply_config_import', params: { source: source.trim(), preview_id: preview!.preview_id } });
      if (generation.current !== requested) return;
      setConfirmed(false);
      if (kind === 'preview') {
        if (result.type !== 'config_import_preview') throw new Error('引擎未返回导入预览');
        setPreview(result.data);
      } else {
        if (result.type !== 'account_config') throw new Error('引擎未返回导入结果，请读取最新配置确认');
        setPreview(null); onImported(result.data);
        setMessage('旧配置已导入，原文已归档。请核对当前账号、应用组和截图设置，再运行任务。');
      }
    } catch (reason) {
      if (generation.current !== requested) return;
      setPreview(null); setConfirmed(false); setError(errorMessage(reason));
    } finally {
      if (generation.current === requested) { working.current = false; setBusy(false); onBusy(false); }
    }
  }

  return <Card aria-label="旧配置导入">
    <div className="card-title"><h2>导入旧配置</h2><Badge appearance="outline">先预览再导入</Badge></div>
    <p>选择旧程序 config 目录中的 one_dragon.yml，或填写旧程序根目录。账号、区服、应用组、兼容的截图设置、任务快捷键及结束后动作会转换，其余 YAML 原文只归档到当前数据目录。</p>
    <Field label="旧程序目录"><Input value={source} disabled={disabled} onChange={(_, data) => changeSource(data.value)} placeholder="例如 E:\旧版一条龙"/></Field>
    <div className="button-row">
      <Button disabled={disabled} onClick={() => void perform('pick')}>选择 one_dragon.yml</Button>
      <Button disabled={disabled || !source.trim()} onClick={() => void perform('preview')}>预览旧配置</Button>
      {busy && <Spinner size="tiny" label="正在处理旧配置"/>}
    </div>
    {current && current.revision !== 0 && <p>当前目录已有正式配置，首批导入不会覆盖它。首次导入须在保存默认账号或通用设置前进行。</p>}
    {error && <MessageBar intent="error"><MessageBarBody>{error}。若导入结果未确认，请先读取最新配置。</MessageBarBody></MessageBar>}
    {message && <MessageBar intent="success"><MessageBarBody>{message}</MessageBarBody></MessageBar>}
    {preview && <section aria-label="旧配置预览">
      <p>账号 {preview.config.instances.length} 个，归档文件 {preview.files.length} 个；默认截图：{captureNames[preview.config.general.capture_method]}。</p>
      <p>任务快捷键：暂停 / 继续 {preview.config.general.shortcuts.pause_resume?.toUpperCase() ?? '关闭'}，停止 {preview.config.general.shortcuts.stop?.toUpperCase() ?? '关闭'}；不启动新任务。</p>
      <p>结束后动作：{afterDoneName(preview.config.general.after_done)}；正常完成后等待 60 秒，可取消。</p>
      {preview.config.instances.map(instance => <div key={instance.id}>
        <h3>{instance.name}{instance.id === preview.config.active_instance ? '（当前账号）' : ''}</h3>
        <p>编号 {instance.id} · {gameRegionOptions.find(option => option.value === instance.game_region)?.label} · {instance.active_in_od ? '参与一条龙' : '不参与一条龙'}</p>
        <p className="path-value">游戏启动：{gameLaunchSummary(instance.game_launch)}</p>
        <p>运行结果提醒：{runNotificationName(instance.run_notifications)}</p>
        {instance.groups.map(group => <details key={group.id}>
          <summary>{group.id === 'one_dragon' ? '一条龙' : group.id} · {group.apps.filter(app => app.enabled).length} 项启用 / {group.apps.length} 项</summary>
          <p>咖啡默认值：{coffeeSummary(group.coffee)}。旧动态选单未转换，请导入后核对。</p>
          <ol>{group.apps.map(app => <li key={app.app_id}>{apps.find(item => item.id === app.app_id)?.name ?? app.app_id} · {app.enabled ? '启用' : '禁用'}</li>)}</ol>
        </details>)}
      </div>)}
      {preview.warnings.map((warning, index) => <p key={index}>{warning}</p>)}
      <details><summary>查看归档文件与迁移范围</summary><ul>{preview.files.map(file => <li key={file.path}>{file.path}：{file.disposition}</li>)}</ul></details>
      {!canApply && <MessageBar intent="warning"><MessageBarBody>当前配置已保存，不能覆盖。预览未修改任何配置。</MessageBarBody></MessageBar>}
      <Checkbox label="我已核对预览，确认将旧配置导入当前数据目录并归档原文" checked={confirmed} disabled={disabled || !canApply} onChange={(_, data) => setConfirmed(data.checked === true)}/>
      <Button appearance="primary" disabled={disabled || !canApply || !confirmed} onClick={() => void perform('apply')}>确认导入旧配置</Button>
    </section>}
  </Card>;
}
