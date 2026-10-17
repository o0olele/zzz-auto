import { useEffect, useState } from 'react';
import { Badge, Button, Card, Field, Input, MessageBar, MessageBarBody, Select, Spinner } from '@fluentui/react-components';
import type { AccountConfig, CoffeeConfig, CoffeeKind } from './generated/protocol';
import { coffeeDefaultConfig, coffeeSettingsFields } from './generated/protocol';
import { errorMessage, request } from './bridge';

export const coffeeName = (value: CoffeeKind) => coffeeSettingsFields[0].options.find(option => option.value === value)?.label ?? value;
export const coffeeSummary = (value: CoffeeConfig = coffeeDefaultConfig) => `${coffeeName(value.kind)} · 超时 ${value.timeout_ms} ms · 启动 / 继续延时 ${value.start_delay_ms} ms`;

export function CoffeeSettings({ settings, disabled, onSave }: { settings: CoffeeConfig; disabled: boolean; onSave: (settings: CoffeeConfig) => void }) {
  const [kind, setKind] = useState(settings.kind);
  const [timeout, setTimeout] = useState(String(settings.timeout_ms));
  const [delay, setDelay] = useState(String(settings.start_delay_ms));
  const draft = { kind, timeout_ms: Number(timeout), start_delay_ms: Number(delay) };
  const value = (key: 'timeout_ms' | 'start_delay_ms') => key === 'timeout_ms' ? timeout : delay;
  const valid = coffeeSettingsFields.every(field => field.kind === 'select' || (value(field.key).trim() !== '' && Number.isSafeInteger(Number(value(field.key))) && Number(value(field.key)) >= field.min && Number(value(field.key)) <= field.max));
  const changed = draft.kind !== settings.kind || draft.timeout_ms !== settings.timeout_ms || draft.start_delay_ms !== settings.start_delay_ms;
  return <Card aria-label="应用组咖啡设置">
    <div className="card-title"><h2>咖啡设置</h2><Badge appearance="outline">按账号与应用组保存</Badge></div>
    <p>路线为澄辉坪 → 汀曼咖啡。保存后可在原生诊断中采用本组参数，完整咖啡流程仍待实机验收。</p>
    <div className="input-fields">{coffeeSettingsFields.map(field => <Field key={field.key} label={field.label}>
      {field.kind === 'select' ? <Select value={kind} disabled={disabled} onChange={(_, data) => setKind(data.value as CoffeeKind)}>{field.options.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}</Select>
        : <Input type="number" min={field.min} max={field.max} value={value(field.key)} disabled={disabled} onChange={(_, data) => field.key === 'timeout_ms' ? setTimeout(data.value) : setDelay(data.value)}/>}
    </Field>)}</div>
    <Button disabled={disabled || !valid || !changed} onClick={() => onSave(draft)}>保存咖啡设置</Button>
  </Card>;
}

export function CoffeePresets({ connected, disabled, onApply, onInvalidate }: {
  connected: boolean; disabled: boolean; onApply: (value: CoffeeConfig) => void; onInvalidate: () => void;
}) {
  const [config, setConfig] = useState<AccountConfig | null>(null);
  const [instanceId, setInstanceId] = useState(0);
  const [groupId, setGroupId] = useState('');
  const [refresh, setRefresh] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    let alive = true;
    setConfig(null); setError(''); setLoading(connected);
    if (connected) request({ type: 'account_config' }).then(result => {
      if (!alive) return;
      if (result.type !== 'account_config') throw new Error('引擎未返回账号配置');
      const instance = result.data.instances.find(item => item.id === result.data.active_instance);
      if (!instance?.groups.length) throw new Error('未找到当前账号的应用组，请读取最新配置');
      setConfig(result.data); setInstanceId(instance.id); setGroupId(instance.groups[0].id);
    }).catch(reason => { if (alive) setError(errorMessage(reason)); })
      .finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, [connected, refresh]);
  const instance = config?.instances.find(item => item.id === instanceId);
  const group = instance?.groups.find(item => item.id === groupId);
  return <section aria-label="读取应用组咖啡参数">
    <p>可采用已保存的组配置，也可手动填写本次参数。采用参数不切换游戏账号，临时改动不回写配置。</p>
    <div className="input-fields">
      <Field label="咖啡参数账号"><Select value={instanceId} disabled={disabled || loading || !config} onChange={(_, data) => {
        const selected = config?.instances.find(item => item.id === Number(data.value));
        setInstanceId(Number(data.value)); setGroupId(selected?.groups[0]?.id ?? ''); onInvalidate();
      }}>{config?.instances.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}</Select></Field>
      <Field label="咖啡参数应用组"><Select value={groupId} disabled={disabled || loading || !instance} onChange={(_, data) => { setGroupId(data.value); onInvalidate(); }}>{instance?.groups.map(item => <option key={item.id} value={item.id}>{item.name ?? (item.id === 'one_dragon' ? '一条龙' : item.id)}</option>)}</Select></Field>
    </div>
    {group && <p>已保存（修订 {config!.revision}）：{coffeeSummary(group.coffee)}</p>}
    <div className="button-row">
      <Button disabled={disabled || loading || !group} onClick={() => { onApply(group!.coffee ?? coffeeDefaultConfig); onInvalidate(); }}>采用所选组咖啡配置</Button>
      <Button disabled={!connected || disabled || loading} onClick={() => { onInvalidate(); setRefresh(current => current + 1); }}>重新读取咖啡配置</Button>
    </div>
    {loading && <Spinner size="tiny" label="读取已保存咖啡配置"/>}
    {error && <MessageBar intent="error"><MessageBarBody>读取咖啡配置失败：{error}。可重试读取或手动填写本次参数。</MessageBarBody></MessageBar>}
  </section>;
}
