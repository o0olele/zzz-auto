import { useEffect, useRef, useState } from 'react';
import { Badge, Button, Card, Field, Input, MessageBar, MessageBarBody, Select, Spinner } from '@fluentui/react-components';
import type { ScreenSampleItem, ScreenSamplePage, ScreenSampleQuery, ScreenSampleView } from './generated/protocol';
import { screenSampleKindOptions } from './generated/protocol';
import { desktop, errorMessage, openSavedScreenSample, screenSampleLibrary } from './bridge';
import { captureNames } from './captureSettings';

const emptyQuery = (): ScreenSampleQuery => ({ kind: null, text: '', before: null });
const kindName = (item: ScreenSampleItem) => screenSampleKindOptions.find(option => option.value === item.kind)?.label ?? item.kind;
const capturedTime = (item: ScreenSampleItem) => new Date(item.captured_at).toLocaleString('zh-CN', { hour12: false });

export function ScreenSampleLibrary({ savedId }: { savedId?: string }) {
  const [filters, setFilters] = useState<ScreenSampleQuery>(emptyQuery);
  const [navigation, setNavigation] = useState({ query: emptyQuery(), cursors: [null] as (string | null)[] });
  const [page, setPage] = useState<ScreenSamplePage | null>(null);
  const [view, setView] = useState<ScreenSampleView | null>(null);
  const [dirty, setDirty] = useState(false);
  const [loading, setLoading] = useState(false);
  const [opening, setOpening] = useState(false);
  const [error, setError] = useState('');
  const [decodeError, setDecodeError] = useState(false);
  const generation = useRef(0);
  const lastSavedId = useRef(savedId);
  const invalidText = [...filters.text].length > 240 || /[\u0000-\u001f\u007f-\u009f]/.test(filters.text);

  useEffect(() => {
    const ticket = ++generation.current;
    setPage(null); setView(null); setError(''); setOpening(false); setDecodeError(false);
    if (!desktop) return;
    setLoading(true);
    screenSampleLibrary(navigation.query).then(result => {
      if (generation.current === ticket) setPage(result);
    }).catch(reason => { if (generation.current === ticket) setError(errorMessage(reason)); })
      .finally(() => { if (generation.current === ticket) setLoading(false); });
    return () => { generation.current++; };
  }, [navigation]);

  useEffect(() => {
    if (savedId && savedId !== lastSavedId.current) {
      generation.current++; setPage(null); setView(null); setDirty(false);
      setNavigation({ query: emptyQuery(), cursors: [null] });
      setFilters(emptyQuery());
    }
    lastSavedId.current = savedId;
  }, [savedId]);

  function edit(next: ScreenSampleQuery) {
    generation.current++; setFilters(next); setDirty(true); setPage(null); setView(null);
    setLoading(false); setOpening(false); setError(''); setDecodeError(false);
  }

  function query() {
    generation.current++; setDirty(false); setPage(null); setView(null); setOpening(false);
    setNavigation({ query: { ...filters, before: null }, cursors: [null] });
  }

  async function open(item: ScreenSampleItem) {
    const ticket = ++generation.current;
    setOpening(true); setView(null); setError(''); setDecodeError(false);
    try {
      const result = await openSavedScreenSample(item.key, item.metadata_sha256);
      if (generation.current !== ticket) return;
      if (result.item.key !== item.key || result.item.metadata_sha256 !== item.metadata_sha256 || result.preview.id !== item.id || result.preview.sha256 !== item.sha256) throw new Error('返回的素材与所选项不匹配，请刷新列表');
      setView(result);
    } catch (reason) { if (generation.current === ticket) setError(errorMessage(reason)); }
    finally { if (generation.current === ticket) setOpening(false); }
  }

  return <Card className="history-browser screen-sample-library" aria-label="已保存素材">
    <div className="card-title"><h2>已保存素材</h2><Button disabled={!desktop || invalidText} onClick={query}>刷新素材列表</Button></div>
    <p>查看本机留档。可在引擎断线或运行中查看，重启后仍保留；当前采集预览独立保存。</p>
    <div className="input-fields">
      <Field label="素材状态筛选"><Select disabled={!desktop} value={filters.kind ?? ''} onChange={(_, data) => edit({ ...filters, kind: data.value as ScreenSampleQuery['kind'] || null })}>
        <option value="">全部画面状态</option>{screenSampleKindOptions.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}
      </Select></Field>
      <Field label="素材备注筛选" validationState={invalidText ? 'error' : 'none'} validationMessage={invalidText ? '筛选备注最多 240 字且不能包含控制字符' : undefined}><Input disabled={!desktop} value={filters.text} onChange={(_, data) => edit({ ...filters, text: data.value })} placeholder="按人工备注查找"/></Field>
    </div>
    <div className="button-row"><Button appearance="primary" disabled={!desktop || invalidText} onClick={query}>查询素材</Button>{dirty && <span>筛选已更改，请查询素材。</span>}</div>
    {loading && <Spinner size="tiny" label="读取素材列表"/>}{opening && <Spinner size="tiny" label="校验并打开素材"/>}
    {error && <MessageBar intent="error"><MessageBarBody>{error}</MessageBarBody></MessageBar>}
    {!!page?.unreadable && <MessageBar intent="warning"><MessageBarBody>{page.unreadable} 项素材损坏、缺失或格式不兼容，原文件已保留。</MessageBarBody></MessageBar>}
    {page && <>
      <Badge appearance="outline">筛选匹配 {page.matching} 张</Badge>
      {!page.items.length && <p>没有符合筛选条件的已保存素材。</p>}
      <div className="sample-library-items">{page.items.map(item => <article key={item.key} className="sample-library-item">
        <div><strong>{kindName(item)}</strong><time>{capturedTime(item)} · {captureNames[item.capture_method]}</time><p>{item.note || '未填写备注'}</p><small>{(item.bytes / 1024).toFixed(1)} KiB · {item.near_black ? '疑似黑帧 · ' : ''}打开时完整校验</small></div>
        <Button disabled={loading || opening} onClick={() => void open(item)}>查看素材</Button>
      </article>)}</div>
      <div className="log-pagination">
        <Button disabled={navigation.cursors.length <= 1 || loading || opening} onClick={() => {
          const cursors = navigation.cursors.slice(0, -1);
          setNavigation({ query: { ...navigation.query, before: cursors[cursors.length - 1] }, cursors });
        }}>上一页素材</Button>
        <span>第 {navigation.cursors.length} 页 · 本页 {page.items.length} 张 · 每页最多 20 张</span>
        <Button disabled={!page.next || loading || opening} onClick={() => {
          if (page.next) setNavigation({ query: { ...navigation.query, before: page.next }, cursors: [...navigation.cursors, page.next] });
        }}>下一页素材</Button>
      </div>
    </>}
    {view && <section className="sample-library-view" aria-label="历史素材回看">
      <div className="card-title"><h3>历史素材回看</h3><Button onClick={() => { generation.current++; setView(null); }}>关闭历史预览</Button></div>
      <p>{kindName(view.item)} · {capturedTime(view.item)} · {view.item.note || '未填写备注'}</p>
      {view.item.near_black && <MessageBar intent="warning"><MessageBarBody>这张留档疑似黑帧，请人工核对画面。</MessageBarBody></MessageBar>}
      {decodeError && <MessageBar intent="error"><MessageBarBody>历史素材预览解码失败，请刷新列表后重新打开。</MessageBarBody></MessageBar>}
      <div className="preview"><img key={view.item.key} alt="已保存的游戏画面素材" src={`data:image/png;base64,${view.preview.screenshot.png_base64}`} onError={() => setDecodeError(true)}/></div>
      <p className="resource-path">SHA-256：{view.item.sha256}</p>
      <Field label="已保存图片路径"><Input readOnly value={view.image_path}/></Field>
      <Field label="已保存元信息路径"><Input readOnly value={view.metadata_path}/></Field>
      <p>已核对文件哈希和 1920×1080 尺寸。状态与备注为人工标注，识别、领奖及实机验收仍需分别确认。</p>
    </section>}
  </Card>;
}
