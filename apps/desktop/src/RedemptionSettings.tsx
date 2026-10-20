import { useState } from 'react';
import { Badge, Button, Card, Field, Input, MessageBar, MessageBarBody } from '@fluentui/react-components';
import type { RedemptionConfig, RedemptionProgress, RedemptionResult, RedemptionOutcome } from './generated/protocol';
import { redemptionSampleCodes } from './generated/protocol';

const outcomes: Record<RedemptionOutcome, string> = { redeemed: '兑换成功', already_redeemed: '已兑换', invalid: '无效', expired: '过期' };
export function RedemptionResults({ results }: { results: RedemptionResult[] }) {
  return <ul aria-label="逐码结果">{results.map(result => <li key={result.code}>{result.code} · {outcomes[result.outcome]}</li>)}</ul>;
}
export function RedemptionStatus({ progress }: { progress: RedemptionProgress }) {
  const pending = progress.codes[progress.index];
  return <section aria-label="兑换码进度">
    <p>已确认 {progress.results.length} / {progress.codes.length} 个码
      {pending && progress.results.length === progress.index ? ` · ${pending.code}：${progress.submitted ? '已尝试提交，结果未确认' : progress.input_verified ? '输入已核对，等待提交' : '等待输入核对'}` : ''}</p>
    <RedemptionResults results={progress.results}/>
  </section>;
}
export function RedemptionConfigSummary({ settings }: { settings?: RedemptionConfig }) {
  const entries = Object.entries(settings?.user_codes ?? {});
  return <details><summary>用户兑换码：{entries.length} 个（所有账号共用）</summary>
    <ul>{entries.map(([code, date]) => <li key={code}>{code} · 有效至 {date}</li>)}</ul>
  </details>;
}
const validDate = (value: string) => /^\d{4}-\d{2}-\d{2}$/.test(value)
  && !Number.isNaN(Date.parse(`${value}T00:00:00Z`))
  && new Date(`${value}T00:00:00Z`).toISOString().slice(0, 10) === value;
const ordered = (codes: RedemptionConfig['user_codes']) => JSON.stringify(Object.entries(codes).sort(([a], [b]) => a.localeCompare(b)));
export function RedemptionSettings({ settings, disabled, onSave }: {
  settings: RedemptionConfig; disabled: boolean; onSave: (settings: RedemptionConfig) => void;
}) {
  const [rows, setRows] = useState(() => Object.entries(settings.user_codes).map(([code, date], id) => ({ id, code, date: date ?? '' })));
  const [nextId, setNextId] = useState(rows.length);
  const codes = Object.fromEntries(rows.map(row => [row.code, row.date]));
  const duplicate = Object.keys(codes).length !== rows.length;
  const overLimit = new Set([...Object.keys(redemptionSampleCodes), ...Object.keys(codes)]).size > 256;
  const valid = !duplicate && !overLimit && rows.every(row => /^[A-Z0-9_-]{1,64}$/.test(row.code) && validDate(row.date));
  const changed = ordered(codes) !== ordered(settings.user_codes);
  return <Card aria-label="全局兑换码设置">
    <div className="card-title"><h2>兑换码</h2><Badge appearance="outline">所有账号共用</Badge></div>
    <p>有效期包含截止当天。同名用户码覆盖内置有效期；按账号跳过已有明确结果或旧版已处理记录的码。当天新增有效码仍可运行。</p>
    <details><summary>内置码表（{Object.keys(redemptionSampleCodes).length} 个）</summary>
      <ul>{Object.entries(redemptionSampleCodes).map(([code, date]) => <li key={code}>{code} · 有效至 {date}</li>)}</ul>
      <p>码表来自随附资源；游戏实际返回决定结果。可添加同名用户码调整有效期，设为过去日期可停止尝试。</p>
    </details>
    {rows.map((row, index) => <div className="input-fields" key={row.id}>
      <Field label={`兑换码 ${index + 1}`}><Input value={row.code} disabled={disabled} maxLength={64}
        onChange={(_, data) => setRows(current => current.map(item => item.id === row.id ? { ...item, code: data.value.trim().toUpperCase() } : item))}/></Field>
      <Field label={`有效期 ${index + 1}`}><Input type="date" value={row.date} disabled={disabled}
        onChange={(_, data) => setRows(current => current.map(item => item.id === row.id ? { ...item, date: data.value } : item))}/></Field>
      <Button disabled={disabled} onClick={() => setRows(current => current.filter(item => item.id !== row.id))}>移除兑换码 {index + 1}</Button>
    </div>)}
    {(!valid && rows.length > 0) && <MessageBar intent="warning"><MessageBarBody>{duplicate ? '兑换码不能重复。' : overLimit ? '合并内置码后最多 256 个。' : '请填写 1..64 位英文字母、数字、下划线或连字符，以及有效日期。'}</MessageBarBody></MessageBar>}
    <div className="button-row">
      <Button disabled={disabled || rows.length >= 256} onClick={() => { setRows(current => [...current, { id: nextId, code: '', date: '2099-01-01' }]); setNextId(nextId + 1); }}>添加兑换码</Button>
      <Button appearance="primary" disabled={disabled || !valid || !changed} onClick={() => onSave({ user_codes: codes })}>保存兑换码</Button>
    </div>
  </Card>;
}
