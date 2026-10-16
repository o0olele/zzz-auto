import { useEffect, useRef, useState } from 'react';
import { Badge, Button, Card, Field, Input, MessageBar, MessageBarBody, Select, Spinner } from '@fluentui/react-components';
import type { AccountConfig, CaptureMethod, GeneralSettings as Settings } from './generated/protocol';
import { generalSettingsFields } from './generated/protocol';
import { errorMessage, request } from './bridge';
import { captureHints } from './captureSettings';

const integer = (value: string, min: number, max: number) => value.trim() !== '' && Number.isSafeInteger(Number(value)) && Number(value) >= min && Number(value) <= max;

export function GeneralSettings({ connected, active, blocked, onLoaded }: {
  connected: boolean; active: boolean; blocked: boolean; onLoaded: (settings: Settings) => void;
}) {
  const [config, setConfig] = useState<AccountConfig | null>(null);
  const [values, setValues] = useState<Record<typeof generalSettingsFields[number]['key'], string>>({ capture_method: 'frame_pool', timeout_ms: '', start_delay_ms: '' });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  const generation = useRef(0);
  const disabled = !connected || blocked || busy || active || !config;
  const valid = generalSettingsFields.every(field => field.kind === 'select'
    ? field.options.some(option => option.value === values[field.key])
    : integer(values[field.key], field.min, field.max));

  function accept(value: AccountConfig) {
    setConfig(value); setValues({ capture_method: value.general.capture_method, timeout_ms: String(value.general.mail.timeout_ms), start_delay_ms: String(value.general.mail.start_delay_ms) });
    onLoaded(value.general);
  }

  async function load() {
    const result = await request({ type: 'account_config' });
    if (result.type !== 'account_config') throw new Error('引擎未返回通用设置');
    return result.data;
  }

  useEffect(() => {
    generation.current++;
    let alive = true;
    setConfig(null); setError(''); setMessage(''); setBusy(false);
    if (connected) load().then(value => { if (alive) accept(value); }).catch(reason => { if (alive) setError(errorMessage(reason)); });
    return () => { alive = false; generation.current++; };
  }, [connected, onLoaded]);

  async function save() {
    if (disabled || !valid) return;
    const requested = generation.current;
    setBusy(true); setError(''); setMessage('');
    try {
      const result = await request({ type: 'update_account_config', params: { revision: config.revision, change: { type: 'save_general', settings: { capture_method: values.capture_method as CaptureMethod, mail: { timeout_ms: Number(values.timeout_ms), start_delay_ms: Number(values.start_delay_ms) } } } } });
      if (generation.current !== requested) return;
      if (result.type !== 'account_config') throw new Error('引擎未返回保存结果');
      accept(result.data); setMessage('通用设置已保存。截图默认值在下次连接或获取截图时应用。');
    } catch (reason) {
      if (generation.current !== requested) return;
      if (typeof reason === 'object' && reason && 'code' in reason && reason.code === 'config_conflict') {
        try { const value = await load(); if (generation.current === requested) { accept(value); setError('配置已被其他窗口或进程修改，已加载最新设置。请核对后重新编辑。'); } }
        catch (refreshError) { if (generation.current === requested) { setConfig(null); setError(`配置冲突，读取最新设置失败：${errorMessage(refreshError)}`); } }
      } else setError(errorMessage(reason));
    } finally { if (generation.current === requested) setBusy(false); }
  }

  return <>
    <div className="page-heading"><div><div className="eyebrow">GENERAL SETTINGS</div><h1>通用设置</h1><p>保存到当前引擎的数据目录，所有本地账号共用。</p></div><Button disabled={!connected || blocked || busy} onClick={async () => {
      const requested = generation.current;
      setBusy(true); setError(''); setMessage('');
      try { const value = await load(); if (generation.current === requested) accept(value); } catch (reason) { if (generation.current === requested) { setConfig(null); setError(errorMessage(reason)); } } finally { if (generation.current === requested) setBusy(false); }
    }}>读取最新设置</Button></div>
    {error && <MessageBar intent="error"><MessageBarBody>{error}</MessageBarBody></MessageBar>}
    {message && <MessageBar intent="success"><MessageBarBody>{message}</MessageBarBody></MessageBar>}
    {active && <MessageBar intent="info"><MessageBarBody>任务仍在运行或收尾，请停止任务后再修改通用设置。</MessageBarBody></MessageBar>}
    {!config && <Card><p>{connected ? '正在读取通用设置。读取失败时可点击“读取最新设置”重试。' : '连接桌面引擎后可读取和编辑本地设置。'}</p></Card>}
    <Card><div className="card-title"><h2>截图与邮件运行</h2>{config && <Badge appearance="outline">修订 {config.revision}</Badge>}</div>
      <div className="input-fields">
        {generalSettingsFields.map(field => <Field key={field.key} label={field.label} validationMessage={field.kind === 'integer' && values[field.key] && !integer(values[field.key], field.min, field.max) ? `请输入 ${field.min}..${field.max} 的整数` : undefined}>
          {field.kind === 'select'
            ? <Select value={values[field.key]} disabled={disabled} onChange={(_, data) => { setValues(current => ({ ...current, [field.key]: data.value })); setMessage(''); }}>{field.options.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}</Select>
            : <Input type="number" min={field.min} max={field.max} value={values[field.key]} disabled={disabled} onChange={(_, data) => { setValues(current => ({ ...current, [field.key]: data.value })); setMessage(''); }}/>}</Field>)}
      </div>
      <p>{captureHints[values.capture_method as CaptureMethod]} 原生诊断中的截图选择仅对本次连接生效。</p>
      <p>应用组使用已保存参数；邮件诊断可以临时调整。超时只计算有效运行时间，暂停不计时。</p>
      <div className="button-row"><Button appearance="primary" disabled={disabled || !valid} onClick={() => void save()}>保存通用设置</Button>{busy && <Spinner size="tiny" label="保存设置"/>}</div>
    </Card>
  </>;
}
