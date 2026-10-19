import { Field, Input, Select } from '@fluentui/react-components';
import type { DownloadProxy } from './generated/protocol';

const names = { system: '系统代理', direct: '直连', custom: '自定义 HTTP / HTTPS 代理' };
export function downloadProxySummary(proxy?: DownloadProxy): string {
  return proxy?.mode === 'custom' ? `${names.custom}：${proxy.url}` : names[proxy?.mode ?? 'system'];
}

export function validProxyUrl(value: string): boolean {
  if (value.length > 2048 || value.trim() !== value || /[\u0000-\u001f\u007f-\u009f?#]/.test(value)) return false;
  try {
    const url = new URL(value);
    return ['http:', 'https:'].includes(url.protocol) && !!url.hostname && !url.username && !url.password && url.pathname === '/' && url.port !== '0';
  } catch { return false; }
}

export function DownloadProxyFields({ mode, url, disabled, onMode, onUrl }: {
  mode: DownloadProxy['mode']; url: string; disabled: boolean;
  onMode: (mode: DownloadProxy['mode']) => void; onUrl: (url: string) => void;
}) {
  return <>
    <h3>下载代理</h3>
    <Field label="下载代理方式"><Select value={mode} disabled={disabled} onChange={(_, data) => onMode(data.value as DownloadProxy['mode'])}>
      {Object.entries(names).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
    </Select></Field>
    {mode === 'custom' && <Field label="下载代理地址" validationMessage={validProxyUrl(url) ? undefined : '请填写不含认证、路径参数、查询或片段的 HTTP / HTTPS 地址'}>
      <Input value={url} disabled={disabled} placeholder="http://127.0.0.1:7890" onChange={(_, data) => onUrl(data.value)}/>
    </Field>}
    <p>保存后用于下一次模型准备和运行资源包下载，进行中的下载沿用启动时设置。代理连接失败不会自动改为直连；自定义代理暂不支持用户名和密码。</p>
  </>;
}
