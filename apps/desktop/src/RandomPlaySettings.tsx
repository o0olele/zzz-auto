import { useState } from 'react';
import { Badge, Button, Card, Field, Input, Select } from '@fluentui/react-components';
import type { RandomPlayConfig, RandomPlayProgress, ShopLocation } from './generated/protocol';
import agents from '../../../resources/source/random-play/agents.json';

export const randomPlayDefault: RandomPlayConfig = { transport_point: 'counter', agent_name_1: '随机', agent_name_2: '随机', turn_dx: -5.5 };
const places: Record<ShopLocation, string> = { counter: '录像店 - 柜台', failume: '澄辉坪 - 录像店营业点', buyast: '布亚斯特城区 - 录像店营业点' };
export function randomPlaySummary(settings: RandomPlayConfig = randomPlayDefault) {
  return `${places[settings.transport_point]} · ${settings.agent_name_1} / ${settings.agent_name_2} · ${settings.turn_dx} 像素/度`;
}
export function RandomPlayStatus({ progress }: { progress: RandomPlayProgress }) {
  const result = progress.outcome === 'started' ? '已确认开始营业' : progress.outcome === 'already_running' ? '已在营业'
    : progress.confirmation_attempted ? '已提交营业，等待结果' : progress.video_ready ? '宣传员与录像带已就绪' : '正在准备营业';
  return <section aria-label="录像店经营进度"><p>录像店：{result} · 游戏日期 {progress.game_day}。</p>
    <p>{randomPlaySummary(progress.config)}</p>
    {!!progress.themes.length && <p>本次主题：{progress.themes.join('、')} · 逐项上架 {progress.theme_index}/3。</p>}
  </section>;
}
export function RandomPlaySettings({ settings = randomPlayDefault, disabled, onSave }: {
  settings?: RandomPlayConfig; disabled: boolean; onSave: (settings: RandomPlayConfig) => void;
}) {
  const [draft, setDraft] = useState(settings);
  const [turnDx, setTurnDx] = useState(String(settings.turn_dx));
  const parsed = Number(turnDx);
  const validNames = [draft.agent_name_1, draft.agent_name_2].every(name => name.trim() === name && name.length > 0 && [...name].length <= 40 && !/[\u0000-\u001f\u007f-\u009f]/u.test(name));
  const valid = validNames && turnDx.trim() !== '' && Number.isFinite(parsed) && parsed >= -50 && parsed <= -0.1;
  const changed = JSON.stringify({ ...draft, turn_dx: parsed }) !== JSON.stringify(settings);
  return <Card aria-label="应用组录像店设置">
    <div className="card-title"><h2>录像店经营</h2><Badge appearance="outline">按账号与应用组保存</Badge></div>
    <p>按游戏日期轮换两名宣传员；任意一项为“随机”时，轮换选择列表前两个位置。指定名称依次匹配，滚动后仍找不到则选择当前列表第一个位置。</p>
    <Field label="录像店营业入口"><Select value={draft.transport_point} disabled={disabled} onChange={(_, data) => setDraft(current => ({ ...current, transport_point: data.value as ShopLocation }))}>
      {Object.entries(places).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
    </Select></Field>
    {(['agent_name_1', 'agent_name_2'] as const).map((key, index) => <Field key={key} label={`宣传员 ${index + 1}`}>
      <Input value={draft[key]} disabled={disabled} list="random-play-agents" onChange={(_, data) => setDraft(current => ({ ...current, [key]: data.value }))}/>
    </Field>)}
    <datalist id="random-play-agents"><option value="随机"/>{agents.map(agent => <option key={agent.id} value={agent.name}/>)}</datalist>
    <Field label="鼠标转向系数（像素/度）" hint="柜台与布亚斯特入口会先校正朝向再前进。默认 -5.5；游戏镜头灵敏度改变后可调整。">
      <Input type="number" min={-50} max={-0.1} step={0.1} value={turnDx} disabled={disabled} onChange={(_, data) => setTurnDx(data.value)}/>
    </Field>
    {!valid && <p role="alert">名称须为 1 至 40 个字符且无首尾空白；转向系数须在 -50 至 -0.1 之间。</p>}
    <Button disabled={disabled || !valid || !changed} onClick={() => onSave({ ...draft, turn_dx: parsed })}>保存录像店设置</Button>
  </Card>;
}
