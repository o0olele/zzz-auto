import { useEffect, useRef, useState } from 'react';
import { Button, MessageBar, MessageBarBody } from '@fluentui/react-components';
import type { AfterDoneAction, AfterDoneStatus } from './generated/protocol';
import { afterDoneOptions } from './generated/protocol';
import { cancelAfterDone, errorMessage } from './bridge';

export const afterDoneName = (action: AfterDoneAction | undefined) => afterDoneOptions.find(item => item.value === (action ?? 'none'))?.label ?? '无';

export function AfterDoneBanner({ status, connected, onChange }: { status: AfterDoneStatus | null; connected: boolean; onChange: (value: AfterDoneStatus | null) => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const current = useRef(status?.id); current.current = status?.id;
  useEffect(() => { setBusy(false); setError(''); }, [status?.id]);
  if (!status) return null;
  const cancelable = status.state === 'watching' || status.state === 'counting_down';
  return <MessageBar intent={status.state === 'failed' ? 'error' : status.state === 'counting_down' || status.state === 'executing' ? 'warning' : 'info'} aria-label="结束后动作状态"><MessageBarBody>
    <strong>结束后：{afterDoneName(status.action)}</strong>
    <p>{status.state === 'counting_down' ? `${status.remaining_seconds} 秒后执行，可取消本次动作。` : status.message}</p>
    <p className="run-message">运行 ID：{status.run_id}</p>
    {cancelable && <Button disabled={!connected || busy} onClick={async () => {
      const id = status.id; setBusy(true); setError('');
      try { const value = await cancelAfterDone(id); if (current.current === id) onChange(value); }
      catch (reason) { if (current.current === id) setError(errorMessage(reason)); }
      finally { if (current.current === id) setBusy(false); }
    }}>取消本次结束后动作</Button>}
    {status.state === 'executing' && <p>动作已提交，正在等待 Windows 返回结果。</p>}
    {error && <p role="alert">{error}</p>}
  </MessageBarBody></MessageBar>;
}
