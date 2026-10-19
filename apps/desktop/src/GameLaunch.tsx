import { useEffect, useRef, useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, Input, MessageBar, MessageBarBody, Select, Spinner } from '@fluentui/react-components';
import type { AccountConfig, Command, GameLaunchSettings, Payload } from './generated/protocol';
import { gameLaunchDefaultConfig } from './generated/protocol';
import { desktop, errorMessage, pickGameExecutable, request } from './bridge';

export const gameLaunchSummary = (settings?: GameLaunchSettings) => {
  const value = settings ?? gameLaunchDefaultConfig;
  return `${value.executable || '未设置路径'}；${value.use_arguments ? `1920×1080 · ${value.fullscreen ? '全屏' : '窗口化'}${value.borderless ? ' · 无边框' : ''} · 显示器 ${value.monitor}` : '使用游戏现有窗口设置'}`;
};

export function GameLaunchEditor({ settings = gameLaunchDefaultConfig, disabled, onSave, onPickerBusy }: {
  settings?: GameLaunchSettings; disabled: boolean; onSave: (settings: GameLaunchSettings) => void; onPickerBusy: (value: boolean) => void;
}) {
  const [draft, setDraft] = useState<GameLaunchSettings>(settings);
  const [picking, setPicking] = useState(false);
  const [error, setError] = useState('');
  const generation = useRef(0);
  const working = useRef(false);
  useEffect(() => () => { generation.current++; }, []);
  const changed = JSON.stringify(settings) !== JSON.stringify(draft);
  const path = draft.executable;
  const validPath = !path || (path.length <= 32700 && /^[A-Za-z]:[\\/](?:.*[\\/])?ZenlessZoneZero\.exe$/i.test(path)
    && !/[\u0000-\u001f\u007f-\u009f"<>|?*]/.test(path) && !path.slice(2).includes(':')
    && !path.split(/[\\/]/).some(part => part === '.' || part === '..' || /[. ]$/.test(part)));
  return <Card aria-label="账号游戏启动设置">
    <div className="card-title"><h2>游戏启动设置</h2><Badge appearance="outline">按账号保存</Badge></div>
    <Field label="游戏可执行文件" validationMessage={!validPath ? '请选择绝区零的 ZenlessZoneZero.exe 绝对路径' : undefined}>
      <Input value={draft.executable} disabled={disabled || picking} onChange={(_, data) => { setDraft(current => ({ ...current, executable: data.value })); setError(''); }}/>
    </Field>
    <div className="button-row"><Button disabled={!desktop || disabled || picking} onClick={async () => {
      if (working.current) return;
      working.current = true; const requested = generation.current;
      setPicking(true); onPickerBusy(true); setError('');
      try { const path = await pickGameExecutable(); if (generation.current === requested && path) setDraft(current => ({ ...current, executable: path })); }
      catch (reason) { if (generation.current === requested) setError(errorMessage(reason)); }
      finally { working.current = false; onPickerBusy(false); if (generation.current === requested) setPicking(false); }
    }}>选择游戏程序</Button>{picking && <Spinner size="tiny" label="选择游戏路径"/>}</div>
    <Checkbox label="使用固定 1080p 启动参数" checked={draft.use_arguments} disabled={disabled || picking} onChange={(_, data) => setDraft(current => ({ ...current, use_arguments: data.checked === true }))}/>
    <div className="input-fields">
      <Field label="游戏窗口模式"><Select value={draft.fullscreen ? '1' : '0'} disabled={disabled || picking || !draft.use_arguments} onChange={(_, data) => setDraft(current => ({ ...current, fullscreen: data.value === '1' }))}><option value="0">窗口化</option><option value="1">全屏</option></Select></Field>
      <Field label="游戏显示器"><Select value={draft.monitor} disabled={disabled || picking || !draft.use_arguments} onChange={(_, data) => setDraft(current => ({ ...current, monitor: Number(data.value) }))}>{[1, 2, 3, 4].map(value => <option key={value} value={value}>{value}</option>)}</Select></Field>
    </div>
    <Checkbox label="无边框窗口" checked={draft.borderless} disabled={disabled || picking || !draft.use_arguments} onChange={(_, data) => setDraft(current => ({ ...current, borderless: data.checked === true }))}/>
    <p>启动参数启用后固定为 1920×1080。保存不会启动游戏；在总览核对当前账号后手动启动，再完成登录和连接。</p>
    {error && <MessageBar intent="error"><MessageBarBody>{error}</MessageBarBody></MessageBar>}
    <Button disabled={disabled || picking || !changed || !validPath} onClick={() => onSave(draft)}>保存游戏启动设置</Button>
  </Card>;
}

export function LaunchGameCard({ connected, blocked, execute }: { connected: boolean; blocked: boolean; execute: (command: Command) => Promise<Payload> }) {
  const [config, setConfig] = useState<AccountConfig | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [loading, setLoading] = useState(false);
  const [launching, setLaunching] = useState(false);
  const [confirmed, setConfirmed] = useState(false);
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  const generation = useRef(0);
  const working = useRef(false);
  useEffect(() => {
    const requested = ++generation.current;
    setConfig(null); setConfirmed(false); setError(''); setMessage(''); setLoading(connected); setLaunching(false); working.current = false;
    if (connected) request({ type: 'account_config' }).then(result => {
      if (generation.current !== requested) return;
      if (result.type !== 'account_config') throw new Error('引擎未返回账号配置');
      setConfig(result.data);
    }).catch(reason => { if (generation.current === requested) setError(errorMessage(reason)); })
      .finally(() => { if (generation.current === requested) setLoading(false); });
    return () => { generation.current++; };
  }, [connected, refresh]);
  useEffect(() => { if (blocked) setConfirmed(false); }, [blocked]);
  const instance = config?.instances.find(item => item.id === config.active_instance);
  const disabled = !desktop || !connected || blocked || loading || launching;
  return <Card aria-label="手动启动游戏">
    <div className="card-title"><h2>启动游戏</h2><Badge appearance="outline">{instance?.name ?? '等待账号配置'}</Badge></div>
    {instance && <p className="path-value">{gameLaunchSummary(instance.game_launch)}</p>}
    <p>先在“账号与应用组”保存路径。启动后请手动登录，从下方扫描并连接游戏窗口，再进入一条龙运行。</p>
    <Checkbox label="确认按当前账号保存的路径与参数启动游戏" checked={confirmed} disabled={disabled || !instance?.game_launch?.executable} onChange={(_, data) => setConfirmed(data.checked === true)}/>
    <div className="button-row"><Button appearance="primary" disabled={disabled || !confirmed || !instance?.game_launch?.executable} onClick={async () => {
      if (!config || !instance || working.current) return;
      working.current = true; const requested = generation.current;
      setLaunching(true); setConfirmed(false); setMessage(''); setError('');
      try {
        const result = await execute({ type: 'launch_game', params: { instance_id: instance.id, revision: config.revision, confirm: true } });
        if (generation.current !== requested) return;
        if (result.type !== 'game_launched') throw new Error('无法确认启动结果，请先检查游戏是否已打开');
        setMessage(`Windows 已接受启动，进程 ${result.data.pid}。请完成登录后手动连接游戏。`);
      } catch (reason) { if (generation.current === requested) { setError(errorMessage(reason)); setConfig(null); } }
      finally { working.current = false; if (generation.current === requested) setLaunching(false); }
    }}>启动当前账号游戏</Button>
      <Button disabled={disabled} onClick={() => setRefresh(value => value + 1)}>重新读取启动设置</Button>
      {(loading || launching) && <Spinner size="tiny" label={launching ? '等待 Windows 启动结果' : '读取启动设置'}/>}</div>
    {error && <MessageBar intent="error"><MessageBarBody>{error}。请检查游戏状态，并重新读取启动设置后核对。</MessageBarBody></MessageBar>}
    {message && <MessageBar intent="success"><MessageBarBody>{message}</MessageBarBody></MessageBar>}
  </Card>;
}
