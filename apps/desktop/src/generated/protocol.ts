// 由 od-tools types 生成；请修改 Rust 协议。
export type ExplorationTarget = "observation" | "cargo";
export type ExplorationProgress = { target: ExplorationTarget, observations: number, cargo: number, scan_finished: boolean, interaction_attempted: boolean, };
export type WorldPatrolConfig = { auto_battle: string, route_list: string, daily_loop_count: number, loop_interval_seconds: number, route_retry_times: number, route_retry_action: PatrolRetryAction, ui_disappear_seconds: number, timeout_ms: number, };
export type PatrolRetryAction = "skip_on_stuck_again" | "retry_on_stuck_again";
export type PatrolProgress = { config: WorldPatrolConfig, routes: Array<PatrolRoute>, completed_rounds: number, route_index: number, point_index: number, attempt: number, finished: Array<string>, failed: Array<string>, round_started_ms: number, next_round_ms: number | null, };
export type PatrolRoute = { id: string, area_id: string, tp_name: string, points: Array<PatrolPoint>, };
export type PatrolPoint = { x: number, y: number, };
export const worldPatrolDefaults = {"auto_battle":"全配队通用","route_list":"","daily_loop_count":1,"loop_interval_seconds":1800,"route_retry_times":1,"route_retry_action":"skip_on_stuck_again","ui_disappear_seconds":10,"timeout_ms":14400000} as WorldPatrolConfig;
export const worldPatrolRouteLists = [{"name":"中央制造区","list_type":"whitelist","route_items":["production_area_building_east_side_1","production_area_building_east_side_2","production_area_building_east_side_3","production_area_building_east_side_4","production_area_building_east_side_5","production_area_building_west_side_1","production_area_building_west_side_2","production_area_rooftop_1"]},{"name":"科研院旧址","list_type":"whitelist","route_items":["former_research_center_1","former_research_center_2","former_research_center_3","former_research_center_4"]},{"name":"粗加工中心","list_type":"whitelist","route_items":["rough_machining_center_1","rough_machining_center_2"]},{"name":"职工社区旧址","list_type":"whitelist","route_items":["former_employee_community_1","former_employee_community_2","former_employee_community_3","former_employee_community_4","former_employee_community_5","former_employee_community_6"]}] as const;
export const worldPatrolRoutes = [{"id":"production_area_building_east_side_1","area_id":"production_area_building_east_side","tp_name":"中央制造区入口","points":[{"x":613,"y":2046},{"x":571,"y":1994},{"x":551,"y":1955},{"x":546,"y":1871},{"x":544,"y":1844},{"x":544,"y":1810},{"x":626,"y":1804},{"x":634,"y":1807},{"x":634,"y":1761},{"x":650,"y":1737},{"x":679,"y":1768},{"x":658,"y":1792}]},{"id":"production_area_building_east_side_2","area_id":"production_area_building_east_side","tp_name":"物流通道","points":[{"x":903,"y":1520},{"x":925,"y":1615},{"x":894,"y":1623},{"x":903,"y":1559},{"x":876,"y":1506},{"x":867,"y":1436},{"x":867,"y":1336},{"x":866,"y":1236},{"x":877,"y":1182},{"x":867,"y":1140},{"x":836,"y":1094},{"x":792,"y":1150}]},{"id":"production_area_building_east_side_3","area_id":"production_area_building_east_side","tp_name":"制造平台","points":[{"x":798,"y":814},{"x":798,"y":750},{"x":832,"y":716},{"x":832,"y":601},{"x":863,"y":560},{"x":863,"y":521},{"x":789,"y":521},{"x":674,"y":534},{"x":597,"y":598},{"x":579,"y":634}]},{"id":"production_area_building_east_side_4","area_id":"production_area_building_east_side","tp_name":"大楼东侧","points":[{"x":465,"y":751},{"x":474,"y":785},{"x":474,"y":841},{"x":492,"y":841},{"x":514,"y":867},{"x":540,"y":858},{"x":577,"y":872},{"x":577,"y":817}]},{"id":"production_area_building_east_side_5","area_id":"production_area_building_east_side","tp_name":"大楼东侧","points":[{"x":410,"y":732},{"x":383,"y":706},{"x":409,"y":671},{"x":409,"y":485},{"x":395,"y":503}]},{"id":"production_area_building_west_side_1","area_id":"production_area_building_west_side","tp_name":"大楼西侧","points":[{"x":1073,"y":398},{"x":1042,"y":398},{"x":1002,"y":417},{"x":1072,"y":429},{"x":1042,"y":465},{"x":932,"y":465},{"x":832,"y":485},{"x":799,"y":515}]},{"id":"production_area_building_west_side_2","area_id":"production_area_building_west_side","tp_name":"通往物流仓储区的入口","points":[{"x":509,"y":639},{"x":683,"y":639},{"x":730,"y":660},{"x":767,"y":674}]},{"id":"production_area_rooftop_1","area_id":"production_area_rooftop","tp_name":"大楼顶楼","points":[{"x":392,"y":349},{"x":375,"y":373},{"x":355,"y":374},{"x":277,"y":373},{"x":277,"y":311},{"x":276,"y":285},{"x":248,"y":255}]},{"id":"former_employee_community_1","area_id":"former_employee_community","tp_name":"职工宿舍北区","points":[{"x":1335,"y":304},{"x":1352,"y":353},{"x":1376,"y":340}]},{"id":"former_employee_community_2","area_id":"former_employee_community","tp_name":"旧商业楼","points":[{"x":1578,"y":898},{"x":1538,"y":899},{"x":1501,"y":899},{"x":1451,"y":836},{"x":1480,"y":906},{"x":1405,"y":826},{"x":1407,"y":807},{"x":1330,"y":806},{"x":1285,"y":770},{"x":1233,"y":739},{"x":1173,"y":753}]},{"id":"former_employee_community_3","area_id":"former_employee_community","tp_name":"职工宿舍南区","points":[{"x":1383,"y":1558},{"x":1315,"y":1565},{"x":1289,"y":1510},{"x":1246,"y":1506},{"x":1271,"y":1544}]},{"id":"former_employee_community_4","area_id":"former_employee_community","tp_name":"职工宿舍西区","points":[{"x":409,"y":1254},{"x":444,"y":1288},{"x":503,"y":1299},{"x":423,"y":1292}]},{"id":"former_employee_community_5","area_id":"former_employee_community","tp_name":"西侧广场","points":[{"x":665,"y":857},{"x":712,"y":863},{"x":768,"y":864},{"x":775,"y":889},{"x":768,"y":919},{"x":810,"y":917},{"x":870,"y":940},{"x":953,"y":940},{"x":959,"y":986},{"x":940,"y":1015},{"x":943,"y":1063},{"x":964,"y":1112},{"x":1022,"y":1116},{"x":1025,"y":1148},{"x":1020,"y":1179},{"x":1032,"y":1234},{"x":1087,"y":1260}]},{"id":"former_employee_community_6","area_id":"former_employee_community","tp_name":"西侧广场","points":[{"x":600,"y":830},{"x":584,"y":776},{"x":578,"y":723},{"x":568,"y":672},{"x":523,"y":658},{"x":474,"y":660},{"x":422,"y":669},{"x":408,"y":704},{"x":390,"y":729},{"x":342,"y":736},{"x":300,"y":742},{"x":263,"y":746}]},{"id":"former_research_center_1","area_id":"former_research_center","tp_name":"科研院西侧","points":[{"x":385,"y":1092},{"x":386,"y":1123},{"x":366,"y":1137},{"x":357,"y":1070}]},{"id":"former_research_center_2","area_id":"former_research_center","tp_name":"科研院内","points":[{"x":747,"y":457},{"x":751,"y":360},{"x":744,"y":309},{"x":757,"y":262},{"x":732,"y":230},{"x":688,"y":248},{"x":633,"y":246},{"x":567,"y":280},{"x":568,"y":323}]},{"id":"former_research_center_3","area_id":"former_research_center","tp_name":"二楼平台","points":[{"x":987,"y":1008},{"x":946,"y":1056},{"x":978,"y":1073},{"x":960,"y":987},{"x":962,"y":923},{"x":975,"y":826},{"x":972,"y":757},{"x":976,"y":736},{"x":965,"y":661},{"x":947,"y":638},{"x":930,"y":621}]},{"id":"former_research_center_4","area_id":"former_research_center","tp_name":"科研院仓库","points":[{"x":311,"y":750},{"x":311,"y":863},{"x":344,"y":925},{"x":352,"y":939}]},{"id":"former_research_center_5","area_id":"former_research_center","tp_name":"一楼广场","points":[{"x":1225,"y":660},{"x":1225,"y":727},{"x":1200,"y":752},{"x":1199,"y":776},{"x":1208,"y":857},{"x":1203,"y":930},{"x":1199,"y":994},{"x":1198,"y":1028},{"x":1204,"y":1150},{"x":1203,"y":1182},{"x":1198,"y":1227},{"x":1145,"y":1224},{"x":1118,"y":1232},{"x":1038,"y":1218}]},{"id":"rough_machining_center_1","area_id":"rough_machining_center","tp_name":"一楼广场","points":[{"x":399,"y":449},{"x":456,"y":510},{"x":455,"y":534},{"x":457,"y":599},{"x":495,"y":596},{"x":531,"y":549}]},{"id":"rough_machining_center_2","area_id":"rough_machining_center","tp_name":"一楼广场","points":[{"x":414,"y":491},{"x":460,"y":443},{"x":519,"y":446},{"x":586,"y":455},{"x":605,"y":413},{"x":605,"y":363},{"x":627,"y":270},{"x":665,"y":310}]}] as const;
export type CombatConfig = { script: string, dodge_script: string, use_merged: boolean, auto_ultimate: boolean, frame_interval_ms: number, directml_device: number | null, mode: InputMode, start_delay_ms: number, timeout_ms: number, max_frame_age_ms: number, tap_ms: number, audio: boolean, audio_threshold_milli: number, dodge_only: boolean, turn_sensitivity_milli: number, gamepad_turn_speed: number, bindings: { [key in string]?: Array<InputProbeAction> }, };
export type CombatScriptCatalog = { revision: string, imported: string | null, battle: Array<string>, dodge: Array<string>, };
export type CombatScriptImportPreview = { source: string, preview_id: string, snapshot_id: string, previous: string | null, battle: Array<string>, dodge: Array<string>, files: number, bytes: number, };
export type CombatProgress = { config: CombatConfig, script_revision: string, state: RunState, inputs_released: boolean, in_battle: boolean, handler: string, action: string, frames: number, stale_frames: number, actions: number, preemptions: number, frame_age_ms: number, audio_correlation: number | null, team: Array<string>, states: { [key in string]?: number }, };
export const combatDefaultConfig = {"script":"全配队通用","dodge_script":"闪避","use_merged":true,"auto_ultimate":false,"frame_interval_ms":20,"directml_device":null,"mode":"foreground","start_delay_ms":3000,"timeout_ms":3600000,"max_frame_age_ms":250,"tap_ms":20,"audio":false,"audio_threshold_milli":100,"dodge_only":true,"turn_sensitivity_milli":1000,"gamepad_turn_speed":16000,"bindings":{}} as const;
export const combatButtonNames = ["闪避","切换角色-下一个","切换角色-上一个","切换后援","普通攻击","特殊攻击","终结技","连携技-左","连携技-右","连携技-取消","移动-前","移动-后","移动-左","移动-右","锁定敌人","快速支援"] as const;
export type ScreenSampleKind = "engagement_daily" | "engagement_confirm" | "engagement_preview" | "coffee_options" | "coffee_energy" | "coffee_buff" | "other";
export type ScreenSampleSource = { session_id: string, hwnd: string, pid: number, capture_method: CaptureMethod, };
export type ScreenSamplePreview = { id: string, source: ScreenSampleSource, screenshot: Screenshot, sha256: string, bytes: number, };
export type ScreenSampleExport = { id: string, kind: ScreenSampleKind, note: string, image_path: string, metadata_path: string, sha256: string, };
export type ScreenSampleState = { preview: ScreenSamplePreview | null, saved: ScreenSampleExport | null, };
export type ScreenSampleQuery = { kind: ScreenSampleKind | null, text: string, before: string | null, };
export type ScreenSampleItem = { key: string, metadata_sha256: string, id: string, kind: ScreenSampleKind, note: string, captured_at: string, capture_method: CaptureMethod, bytes: number, sha256: string, near_black: boolean, };
export type ScreenSamplePage = { items: Array<ScreenSampleItem>, matching: number, unreadable: number, next: string | null, };
export type ScreenSampleView = { item: ScreenSampleItem, preview: ScreenSamplePreview, image_path: string, metadata_path: string, };
export type AfterDoneAction = "none" | "close_game" | "shutdown" | "hibernate" | "sleep";
export type AfterDoneState = "watching" | "counting_down" | "executing" | "succeeded" | "cancelled" | "failed";
export type AfterDoneStatus = { id: string, run_id: string, action: AfterDoneAction, state: AfterDoneState, remaining_seconds: number, message: string, };
export type GameRegion = "cn" | "cn_b" | "us" | "eu" | "asia" | "twhkmo";
export type RunPolicy = "always" | "skip_daily_success";
export type GameDay = { region: GameRegion, date: string, };
export type RunContext = { instance_id: number, instance_name: string, group_id: string, group_name?: string, config_revision: number, app_ids: Array<string>, rerun_completed?: boolean, standalone?: boolean, game_day?: GameDay, };
export type PlanDecision = "disabled" | "run" | "already_done" | "no_work" | "blocked";
export type AppAvailability = "mail_trial" | "coffee_trial" | "daily_trial" | "unmigrated" | "unknown";
export type PlannedApp = { app_id: string, name: string, enabled: boolean, availability: AppAvailability, decision: PlanDecision, completed_run_id: string | null, };
export type SuibianTempleConfig = { auto_manage_enabled: boolean, yum_cha_sin: boolean, yum_cha_sin_period_refresh: boolean, adventure_duration: AdventureDuration, adventure_missions: [string, string, string, string], craft_drag_times: number, good_goods_purchase_enabled: boolean, boo_box_purchase_enabled: boolean, boo_box_adventure_price: BangbooPrice, boo_box_craft_price: BangbooPrice, boo_box_sell_price: BangbooPrice, legacy_pawnshop: LegacyPawnshopConfig, timeout_ms: number, };
export type AdventureDuration = "min_3" | "min_15" | "hour_1" | "hour_2" | "hour_6" | "hour_12" | "hour_20";
export type BangbooPrice = "s1" | "s2" | "s3" | "s4" | "none";
export type LegacyPawnshopConfig = { omnicoin_enabled: boolean, omnicoin_priority: Array<PawnshopOmnicoinGoods>, crest_enabled: boolean, crest_priority: Array<PawnshopCrestGoods>, crest_unlimited_denny_enabled: boolean, };
export type PawnshopOmnicoinGoods = "hifi_master_copy" | "senior_investigator_log" | "w_engine_energy_module" | "ether_plating_agent" | "prepaid_power_card";
export type PawnshopCrestGoods = "bangboo_system_widget" | "denny";
export const chargePlanDefaults = {"plan_list":[],"loop":true,"daily_reset_plan_times":false,"last_daily_reset_day":null,"skip_plan":false,"double_reward":false,"combat_simulation_double_reward_config":{"plan_id":"00000000-0000-0000-0000-000000000000","tab_name":"训练","category_name":"实战模拟室","mission_type_name":"基础材料","mission_name":"调查专项","level":"默认等级","auto_battle_config":"全配队通用","run_times":0,"plan_times":1,"card_num":null,"predefined_team_idx":-1,"notorious_hunt_buff_num":1},"restore_charge":"none"} as ChargePlanConfig;
export const chargePlanCatalog = [{"category_name":"实战模拟室","mission_type_list":[{"mission_type_name":"基础材料","mission_type_name_display":null,"mission_list":[{"mission_name":"调查专项","mission_name_display":"代理人经验"},{"mission_name":"音擎专项","mission_name_display":"音擎经验"},{"mission_name":"赏金专项","mission_name_display":"丁尼"},{"mission_name":"镀剂专项","mission_name_display":"驱动盘经验"}]},{"mission_type_name":"代理人晋升","mission_type_name_display":null,"mission_list":[{"mission_name":"锋御演练","mission_name_display":"锋御"},{"mission_name":"命破演练","mission_name_display":"命破"},{"mission_name":"强攻演练","mission_name_display":"强攻"},{"mission_name":"击破演练","mission_name_display":"击破"},{"mission_name":"异常演练","mission_name_display":"异常"},{"mission_name":"支援演练","mission_name_display":"支援"},{"mission_name":"防护演练","mission_name_display":"防护"}]},{"mission_type_name":"音擎改装","mission_type_name_display":null,"mission_list":[{"mission_name":"锋御耦合试验","mission_name_display":"锋御"},{"mission_name":"命破共鸣试验","mission_name_display":"命破"},{"mission_name":"强攻极片试验","mission_name_display":"强攻"},{"mission_name":"击破振膜试验","mission_name_display":"击破"},{"mission_name":"异常磁体试验","mission_name_display":"异常"},{"mission_name":"支援分频试验","mission_name_display":"支援"},{"mission_name":"防护音圈试验","mission_name_display":"防护"}]},{"mission_type_name":"代理人技能","mission_type_name_display":null,"mission_list":[{"mission_name":"刚性测试","mission_name_display":"物理"},{"mission_name":"升温测试","mission_name_display":"火属性"},{"mission_name":"冰结测试","mission_name_display":"冰属性"},{"mission_name":"电流测试","mission_name_display":"电属性"},{"mission_name":"风切测试","mission_name_display":"风属性"},{"mission_name":"共鸣测试","mission_name_display":"以太"},{"mission_name":"琢光测试","mission_name_display":"流明"}]},{"mission_type_name":"自定义模板","mission_type_name_display":null,"mission_list":[{"mission_name":"自定义卡组1","mission_name_display":null},{"mission_name":"自定义卡组2","mission_name_display":null},{"mission_name":"自定义卡组3","mission_name_display":null},{"mission_name":"自定义卡组4","mission_name_display":null},{"mission_name":"自定义卡组5","mission_name_display":null}]},{"mission_type_name":"代理人方案培养","mission_type_name_display":"特训目标","mission_list":[{"mission_name":"代理人方案培养","mission_name_display":"特训目标"}]}]},{"category_name":"区域巡防","mission_type_list":[{"mission_type_name":"怪兽与怪客","mission_type_name_display":"摇摆爵士 混沌重金属","mission_list":[]},{"mission_type_name":"恶拳与凶枪","mission_type_name_display":"激素朋克 獠牙重金属","mission_list":[]},{"mission_type_name":"猎人与忠犬","mission_type_name_display":"震星迪斯科 雷暴重金属","mission_list":[]},{"mission_type_name":"高塔与巨炮","mission_type_name_display":"啄木鸟电音 灵魂摇滚","mission_list":[]},{"mission_type_name":"疯子与追随者","mission_type_name_display":"河豚电音 炎狱重金属","mission_list":[]},{"mission_type_name":"锐牙与钝斧","mission_type_name_display":"自由蓝调 极地重金属","mission_list":[]},{"mission_type_name":"猎手与凶兽","mission_type_name_display":"混沌爵士 原始朋克","mission_list":[]},{"mission_type_name":"怪兽二重奏","mission_type_name_display":"折枝剑歌 静听嘉音","mission_list":[]},{"mission_type_name":"游侠与守卫","mission_type_name_display":"如影相随 法厄同之歌","mission_list":[]},{"mission_type_name":"口舌与刀兵","mission_type_name_display":"云岿如我 山大王","mission_list":[]},{"mission_type_name":"铁律与狂徒","mission_type_name_display":"拂晓生花 月光骑士颂","mission_list":[]},{"mission_type_name":"诡步与重壁","mission_type_name_display":"沧浪行歌 流光咏叹","mission_list":[]},{"mission_type_name":"伏龙与战车","mission_type_name_display":"雪兔梦游仙境 囚徒手记","mission_list":[]},{"mission_type_name":"机师与叛甲","mission_type_name_display":"呼啸沙龙 拂晓行纪","mission_list":[]},{"mission_type_name":"迅雷与裂爪","mission_type_name_display":"谶羽之誓 荆棘玫瑰","mission_list":[]},{"mission_type_name":"代理人方案培养","mission_type_name_display":"特训目标","mission_list":[]}]},{"category_name":"专业挑战室","mission_type_list":[{"mission_type_name":"恶名·杜拉罕","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"祸首·通缉打手","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"恶名·装甲哈提","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"汉斯·蓄能型","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"「提丰·重击者型」","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"蛮横力士","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"多佩冈亚·简","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"色雷斯人","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"伐木机","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"秽息蚀者·阿瓦鲁斯","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"牲鬼·凶魁愚者","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"秽蚀·狛野真斗","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"秽息行者·蝎骸","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"牲鬼·卫律使者","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"空域戒戍者","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"「征服者」","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"代理人方案培养","mission_type_name_display":"特训目标","mission_list":[]}]},{"category_name":"恶名狩猎","mission_type_list":[{"mission_type_name":"初生死路屠夫","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"未知复合侵蚀体","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"冥宁芙·双子","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"「霸主侵蚀体·庞培」","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"牲鬼·布林格","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"秽息司祭","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"彷徨猎手","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"魇缚者·叶释渊","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"猎血清道夫","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"基塔布鲁","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"库萨里库","mission_type_name_display":null,"mission_list":[]},{"mission_type_name":"代理人方案培养","mission_type_name_display":"特训目标","mission_list":[]}]},{"category_name":"合成电池","mission_type_list":[]}] as Array<ChargeCategory>;
export const chargePlanLevels = ["默认等级","等级Lv.65","等级Lv.60","等级Lv.50","等级Lv.40","等级Lv.30"] as const;
export type RestoreCharge = "none" | "backup_only" | "ether_only" | "both";
export type ChargePlanItem = { plan_id: string, tab_name: string, category_name: string, mission_type_name: string, mission_name: string | null, level: string, auto_battle_config: string, run_times: number, plan_times: number, card_num: number | null, predefined_team_idx: number, notorious_hunt_buff_num: number, };
export type ChargePlanConfig = { plan_list: Array<ChargePlanItem>, loop: boolean, daily_reset_plan_times: boolean, last_daily_reset_day: GameDay | null, skip_plan: boolean, double_reward: boolean, combat_simulation_double_reward_config: ChargePlanItem, restore_charge: RestoreCharge, };
export type ChargeResources = { charge: number, backup: number, ether: number, };
export type ChargeDecision = "disabled" | "completed" | "ready" | "needs_restore" | "insufficient" | "unknown_resources";
export type ChargePreviewItem = { plan_id: string, effective_run_times: number, estimated_charge: number, script: string, decision: ChargeDecision, };
export type ChargePlanPreview = { config: ChargePlanConfig, game_day: GameDay, reset_daily: boolean, rounds_rebased: number, items: Array<ChargePreviewItem>, next_plan_id: string | null, temporary: ChargePlanItem | null, temporary_script: string | null, double_check_pending: boolean, };
export type ChargeGroupPreview = { instance_id: number, group_id: string, revision: number, forecast: ChargePlanPreview, };
export type ChargeCategory = { category_name: string, mission_type_list: Array<ChargeMission>, };
export type ChargeMission = { mission_type_name: string, mission_type_name_display: string | null, mission_list: Array<ChargeMissionName>, };
export type ChargeMissionName = { mission_name: string, mission_name_display: string | null, };
export type IntelBoardConfig = { predefined_team_idx: number, auto_battle_config: string, exp_grind_mode: boolean, };
export type IntelHistory = { week: IntelWeek | null, totals: WeeklyTotals, source_run_id: string | null, unreadable: number, };
export type IntelWeek = { region: GameRegion, monday: string, };
export type WeeklyTotals = { contribution: number,
/**
 * 与旧版相同的历史贡献估计，不是从游戏直接读出的经验值。
 */
estimated_base_exp: number, notorious_hunt_count: number, expert_challenge_count: number, };
export type CommissionKind = "notorious_hunt" | "expert_challenge";
export type SettledCommission = { kind: CommissionKind, result: NormalBattleResult, };
export type PendingCommission = { kind: CommissionKind, deployed: boolean, result: NormalBattleResult | null, };
export type IntelLedger = { config: IntelBoardConfig, week: IntelWeek, baseline: WeeklyTotals, initial_contribution: number | null, observed_contribution: number | null, estimated_base_exp: number, pending: PendingCommission | null, settled: Array<SettledCommission>, };
export type AppCombatPlan = { config: CombatConfig, script_revision: string, };
export type IntelFlow = { stage: string, filter_step: number, filtered: boolean, scrolls: number, failed_accepts: number, candidate_changes: number, transport_attempted: boolean, arrived: boolean, fallback_team: boolean, selection: TeamSelection | null, movement: ChallengeMove | null, outcome: IntelOutcome | null, };
export type IntelOutcome = "weekly_goal" | "no_commissions";
export type TeamSelection = { targets: Array<number>, phase: SelectionPhase, top_scrolls: number, page: number, seen: Array<[number, number | null]>, chosen: Array<number>, cleared: boolean, pending: SelectionAction | null, };
export type SelectionPhase = "reset_clear" | "clear" | "reset_choose" | "choose" | "reset_verify" | "verify" | "ready" | "confirmed";
export type SelectionAction = { "type": "up" } | { "type": "down" } | { "type": "toggle", index: number, order: number | null, } | { "type": "confirm" };
export type ChallengeMove = { expert: boolean, buff_index: number, stage: MoveStage, moves: number, blind_moves: number, turns: number, interactions: number, search_steps: number, interaction_attempted: boolean, buff_attempted: boolean, expert_move_attempted: boolean, expert_remaining_ms: number, last_distance: number | null, boss_frames: number, pending: MoveAction | null, };
export type MoveStage = "approach" | "interact" | "buff" | "advance" | "done";
export type MoveAction = { "type": "forward", ms: number, measured: boolean, } | { "type": "search", key: number, } | { "type": "turn", dx: number, } | { "type": "interact" } | { "type": "buff", rect: [number, number, number, number], };
export type NormalBattleResult = "completed" | "retreat";
export type GroupPlan = { exploration_combat?: AppCombatPlan, world_patrol_config?: WorldPatrolConfig, world_patrol_combat?: AppCombatPlan, world_patrol_checkpoint?: PatrolProgress, world_patrol_active_ms: number, charge_plan_config?: ChargePlanConfig, suibian_temple_config?: SuibianTempleConfig, app_combat?: AppCombatPlan, intel_board_config?: IntelBoardConfig, intel_board_history?: IntelHistory, predefined_teams?: TeamConfig, random_play_config?: RandomPlayConfig, redemption_codes?: Array<RedemptionCode>, redemption_history?: RedemptionHistory, after_done: AfterDoneAction, context: RunContext, mail_config: MailConfig, coffee_config: CoffeeConfig, daily_config?: DailyConfig, dismantle_config?: DismantleConfig, items: Array<PlannedApp>, blockers: Array<string>, executable: boolean, all_done: boolean, game_day: GameDay, run_policy: RunPolicy, warnings: Array<string>, valid_until: string, };
export type HistoryItem = { record: RunRecord, interrupted: boolean, };
export type RunHistory = { instance_id: number, items: Array<HistoryItem>, unreadable: number, has_more: boolean, };
export type HistoryOutcome = "active" | "succeeded" | "failed" | "timed_out" | "stopped" | "interrupted";
export type HistoryCursor = { started_at: string, id: string, };
export type HistoryQuery = { group_id: string | null, app_id: string | null, game_day: string | null, outcome: HistoryOutcome | null, before: HistoryCursor | null, };
export type HistoryPage = { instance_id: number, query: HistoryQuery, items: Array<HistoryItem>, matching: number, unreadable: number, next: HistoryCursor | null, content_id: string, };
export type HistoryExport = { path: string, instance_id: number, record_count: number, content_id: string, };
export type GroupApp = { app_id: string, enabled: boolean, };
export type GeneralSettings = { download_proxy?: DownloadProxy, after_done: AfterDoneAction, capture_method: CaptureMethod, mail: MailConfig, shortcuts: ShortcutSettings, };
export type DownloadProxy = { "mode": "system" } | { "mode": "direct" } | { "mode": "custom", url: string, };
export type ShortcutKey = "f1" | "f2" | "f3" | "f4" | "f5" | "f6" | "f7" | "f8" | "f9" | "f10" | "f11";
export type ShortcutSettings = { pause_resume: ShortcutKey | null, stop: ShortcutKey | null, };
export type ShortcutStatus = { active: ShortcutSettings | null, registration_error: string | null, action_error: string | null, };
export type AppGroup = { world_patrol?: WorldPatrolConfig, charge_plan?: ChargePlanConfig, suibian_temple?: SuibianTempleConfig, intel_board?: IntelBoardConfig, random_play?: RandomPlayConfig, id: string, name?: string, run_policy: RunPolicy, coffee: CoffeeConfig, daily?: DailyConfig, dismantle?: DismantleConfig, apps: Array<GroupApp>, };
export type AccountInstance = { predefined_teams?: TeamConfig, redemption_used?: Array<string>, id: number, name: string, active_in_od: boolean, combat?: CombatConfig, game_region: GameRegion, game_launch?: GameLaunchSettings, run_notifications?: RunNotificationSettings, groups: Array<AppGroup>, };
export type RunNotificationMode = "off" | "errors" | "all";
export type RunNotificationSettings = { mode: RunNotificationMode, };
export type RunNoticeLevel = "success" | "info" | "warning" | "error";
export type RunNotice = { id: string, session_id: string, run_id: string, instance_id: number, instance_name: string, group_name: string, at: string, level: RunNoticeLevel, title: string, message: string, read: boolean, };
export type RunNotifications = { revision: number, items: Array<RunNotice>, dropped: number, };
export type GameLaunchSettings = { executable: string, use_arguments: boolean, fullscreen: boolean, borderless: boolean, monitor: number, };
export type GameLaunchResult = { instance_id: number, pid: number, };
export type AccountConfig = { redemption?: RedemptionConfig, schema: number, revision: number, general: GeneralSettings, active_instance: number, instances: Array<AccountInstance>, import_id: string | null, reserved_instance_ids?: Array<number>, };
export type ConfigBackup = { id: string, revision: number, bytes: number, modified_at: string | null, };
export type ConfigBackups = { revision: number, items: Array<ConfigBackup>, omitted: number, unavailable: number, };
export type ConfigBackupPreview = { id: string, preview_id: string, current: AccountConfig, backup: AccountConfig, };
export type ConfigRecoveryState = "missing" | "damaged" | "healthy" | "unsupported";
export type ConfigRecoveryCurrent = { state: ConfigRecoveryState, bytes: number | null, sha256: string | null, };
export type ConfigRecoveryCatalog = { data_directory: string, current: ConfigRecoveryCurrent, backups: Array<ConfigBackup>, omitted: number, unavailable: number, };
export type ConfigRecoveryPreview = { data_directory: string, id: string, preview_id: string, current: ConfigRecoveryCurrent, recovered: AccountConfig, unreadable_backups: number, warnings: Array<string>, };
export type ConfigRecoveryResult = { config: AccountConfig, preserved_file: string | null, recovered_backup: string, };
export type ConfigRecoveryRequest = { "type": "list" } | { "type": "preview", id: string, } | { "type": "restore", id: string, preview_id: string, confirm: boolean, };
export type ConfigRecoveryJobState = "running" | "succeeded" | "failed";
export type ConfigRecoveryJob = { id: string, request: ConfigRecoveryRequest, state: ConfigRecoveryJobState, phase: string, reconnected: boolean, reconnect_error: string | null, error: string | null, error_code: string | null, };
export type DesktopConfigRecovery = { job: ConfigRecoveryJob | null, catalog: ConfigRecoveryCatalog | null, preview: ConfigRecoveryPreview | null, restored: ConfigRecoveryResult | null, };
export type ConfigChange = { "type": "save_charge_plan", instance_id: number, group_id: string, settings: ChargePlanConfig, } | { "type": "save_suibian_temple", instance_id: number, group_id: string, settings: SuibianTempleConfig, } | { "type": "save_intel_board", instance_id: number, group_id: string, settings: IntelBoardConfig, } | { "type": "save_world_patrol", instance_id: number, group_id: string, settings: WorldPatrolConfig, } | { "type": "add_group_app", instance_id: number, group_id: string, app_id: string, } | { "type": "save_teams", instance_id: number, settings: TeamConfig, } | { "type": "save_random_play", instance_id: number, group_id: string, settings: RandomPlayConfig, } | { "type": "save_redemption", settings: RedemptionConfig, } | { "type": "save_combat", instance_id: number, settings: CombatConfig, } | { "type": "save_dismantle", instance_id: number, group_id: string, settings: DismantleConfig, } | { "type": "save_daily", instance_id: number, group_id: string, settings: DailyConfig, } | { "type": "save_run_notifications", instance_id: number, settings: RunNotificationSettings, } | { "type": "save_game_launch", instance_id: number, settings: GameLaunchSettings, } | { "type": "set_after_done", action: AfterDoneAction, } | { "type": "save_coffee", instance_id: number, group_id: string, settings: CoffeeConfig, } | { "type": "set_game_region", instance_id: number, game_region: GameRegion, } | { "type": "set_run_policy", instance_id: number, group_id: string, run_policy: RunPolicy, } | { "type": "save_general", settings: GeneralSettings, } | { "type": "create_instance", name: string, } | { "type": "copy_instance", instance_id: number, name: string, } | { "type": "delete_instance", instance_id: number, confirm: boolean, } | { "type": "reorder_instances", instance_ids: Array<number>, } | { "type": "reorder_groups", instance_id: number, group_ids: Array<string>, } | { "type": "update_instance", instance_id: number, name: string, active_in_od: boolean, } | { "type": "switch_instance", instance_id: number, } | { "type": "save_group", instance_id: number, group_id: string, apps: Array<GroupApp>, } | { "type": "create_group", instance_id: number, name: string, source_group_id: string | null, } | { "type": "rename_group", instance_id: number, group_id: string, name: string, } | { "type": "delete_group", instance_id: number, group_id: string, };
export type ImportFile = { path: string, sha256: string, disposition: string, };
export type ImportPreview = { preview_id: string, config: AccountConfig, files: Array<ImportFile>, warnings: Array<string>, };
export type AppDefinition = { id: string, name: string, execution: string, reason: string, factory: string, configs: Array<string>, settings: Array<string>, run_records: Array<string>, source_files: Array<string>, priority: number, default_group: boolean, implemented: boolean, };
export type RunState = "running" | "pausing" | "paused" | "stopping" | "succeeded" | "failed" | "stopped";
export type GroupStepState = "pending" | "running" | "skipped" | "succeeded" | "failed" | "stopped";
export type GroupStepProgress = { app_id: string, state: GroupStepState, inputs_released: boolean, started_at: string | null, finished_at: string | null, };
export type GroupProgress = { checkpoint: number, state: RunState, current_index: number | null, steps: Array<GroupStepProgress>, };
export type MailConfig = { timeout_ms: number, start_delay_ms: number, };
export type MailState = "running" | "paused" | "succeeded" | "failed" | "stopped" | "timed_out";
export type MailPhase = "navigate" | "inspect" | "claim" | "after_claim" | "confirm" | "after_confirm" | "return_list" | "return_world" | "done";
export type MailProgress = { config: MailConfig, state: MailState, phase: MailPhase, claim_attempted: boolean, reward_observed: boolean, inputs_released: boolean, active_ms: number, };
export type MapConfig = { timeout_ms: number, start_delay_ms: number, };
export type MapState = "running" | "paused" | "succeeded" | "failed" | "stopped" | "timed_out";
export type MapPhase = "inspect" | "select_area" | "select_point" | "inspect_dialog" | "confirm" | "wait_arrival" | "done";
export type MapProgress = { config: MapConfig, state: MapState, phase: MapPhase, point_attempted: boolean, confirm_attempted: boolean, inputs_released: boolean, active_ms: number, };
export type CoffeeKind = "tinman" | "espresso";
export type CoffeeConfig = { kind: CoffeeKind, timeout_ms: number, start_delay_ms: number, };
export type CoffeeState = "running" | "paused" | "succeeded" | "failed" | "stopped" | "timed_out";
export type CoffeePhase = "inspect" | "order" | "wait_energy" | "confirm_energy" | "after_energy" | "confirm_buff" | "done";
export type CoffeeProgress = { config: CoffeeConfig, state: CoffeeState, phase: CoffeePhase, approach?: CoffeeApproachProgress, order_attempted: boolean, energy_seen: boolean, inputs_released: boolean, active_ms: number, };
export type RedemptionConfig = { user_codes: { [key in string]?: string }, };
export type RedemptionCode = { code: string, expires_on: string, };
export type RedemptionOutcome = "redeemed" | "already_redeemed" | "invalid" | "expired";
export type RedemptionResult = { code: string, outcome: RedemptionOutcome, };
export type RedemptionHistory = { results: Array<RedemptionResult>, unreadable: number, };
export type RedemptionProgress = { codes: Array<RedemptionCode>, index: number, input_verified: boolean, submitted: boolean, results: Array<RedemptionResult>, };
export type DismantleGrade = "b" | "a_and_below" | "s_and_below";
export type DismantleConfig = { grade: DismantleGrade, include_abandoned: boolean, };
export type DismantleProgress = { config: DismantleConfig, filter_reset: boolean, grade_selected: boolean, abandoned_selected: boolean, filter_applied: boolean, dismantle_attempted: boolean, confirmation_attempted: boolean, obtained: boolean, empty_selection: boolean, returned_storage: boolean, };
export type ShopLocation = "counter" | "failume" | "buyast";
export type RandomPlayConfig = { transport_point: ShopLocation, agent_name_1: string, agent_name_2: string, turn_dx: number, };
export type ShopOutcome = "already_running" | "started";
export type RandomPlayProgress = { config: RandomPlayConfig, game_day: string, promoter_ready: boolean, promoter_scrolls: number, themes: Array<string>, theme_index: number, theme_scrolls: number, recommended_attempted: boolean, video_ready: boolean, interaction_attempted: boolean, transport_retries: number, start_attempted: boolean, confirmation_attempted: boolean, outcome: ShopOutcome | null, };
export type PredefinedTeam = { name: string, auto_battle: string, agent_ids: [string, string, string], };
export type TeamConfig = { check_timeout_ms?: number, teams: Array<PredefinedTeam>, };
export type TeamMembers = { index: number, name: string, member_count: number, agent_ids: [string, string, string], };
export type TeamCheckProgress = { config: TeamConfig, pages_read: number, opened: boolean, top_scrolls: number, members: Array<TeamMembers>, conflicted: Array<number>, unreadable: Array<number>, saved_revision: number | null, };
export const teamDefaultConfig = {"check_timeout_ms":300000,"teams":[{"name":"编队1","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]},{"name":"编队2","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]},{"name":"编队3","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]},{"name":"编队4","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]},{"name":"编队5","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]},{"name":"编队6","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]},{"name":"编队7","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]},{"name":"编队8","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]},{"name":"编队9","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]},{"name":"编队10","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]},{"name":"编队11","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]},{"name":"编队12","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]},{"name":"编队13","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]},{"name":"编队14","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]},{"name":"编队15","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]},{"name":"编队16","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]},{"name":"编队17","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]},{"name":"编队18","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]},{"name":"编队19","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]},{"name":"编队20","auto_battle":"全配队通用","agent_ids":["unknown","unknown","unknown"]}]} as TeamConfig;
export type DailyConfig = { timeout_ms: number, start_delay_ms: number, };
export type DailyProgress = { exploration?: ExplorationProgress, world_patrol?: PatrolProgress, temple?: { phase: string, current: string | null, completed: Array<string>, config: SuibianTempleConfig, returned_world: boolean, inputs_released: boolean }, intel_flow?: IntelFlow, app_combat?: AppCombatPlan, intel_battle?: CombatProgress, intel_board?: IntelLedger, team_check?: TeamCheckProgress, random_play?: RandomPlayProgress, redemption?: RedemptionProgress, dismantle?: DismantleProgress, app_id: string, config: DailyConfig, state: MailState, stage: string, attempted: boolean, completion_proven: boolean, returned_world: boolean, inputs_released: boolean, scratch_rows: number, tasks_checked: boolean, levels_selected: boolean, route_confirmed: boolean, activation_attempted: boolean, transport_attempted: boolean, approach_attempted: boolean, reward_checked?: boolean, reward_confirmed?: boolean, reveal_observed?: boolean, active_ms: number, };
export const trialAppOptions = [{"id":"map_exploration","label":"空洞探索"},{"id":"drive_disc_dismantle","label":"驱动盘拆解"},{"id":"world_patrol","label":"锄大地"},{"id":"suibian_temple","label":"随便观"},{"id":"intel_board","label":"情报板"},{"id":"ridu_weekly","label":"丽都周纪 (领奖励)"},{"id":"city_fund","label":"丽都城募"},{"id":"engagement_reward","label":"活跃度奖励"},{"id":"coffee","label":"咖啡店"},{"id":"random_play","label":"录像店营业"},{"id":"hou_hou_bakery","label":"吼吼饼铺"},{"id":"trigrams_collection","label":"卦象集录"},{"id":"scratch_card","label":"刮刮卡"},{"id":"email","label":"邮件"},{"id":"redemption_code","label":"兑换码"},{"id":"predefined_team_checker","label":"预备编队角色识别"}] as const;
export type CoffeeApproachPhase = "inspect_world" | "open_map" | "wait_map" | "select_area" | "select_point" | "wait_dialog" | "confirm" | "wait_arrival" | "interact" | "wait_dialogue" | "advance_dialogue" | "done";
export type CoffeeApproachProgress = { phase: CoffeeApproachPhase, dialogue_advances: number, };
export type InputMode = "foreground" | "foreground_legacy" | "background" | "background_post" | "xbox" | "ds4";
export type InputSide = "left" | "right";
export type MouseButton = "left" | "right" | "middle" | "x1" | "x2";
export type InputProbeAction = { "type": "key", key: number, } | { "type": "mouse_button", button: MouseButton, x: number, y: number, } | { "type": "stick", stick: InputSide, x: number, y: number, } | { "type": "trigger", trigger: InputSide, value: number, };
export type InputProbeConfig = { mode: InputMode, action: InputProbeAction, hold_ms: number, start_delay_ms: number, };
export type InputProbeResult = { config: InputProbeConfig, pressed: boolean, released: boolean, stopped: boolean, enqueue_us: number, key_down_ms: number, held_ms: number, release_ms: number, stop_ms: number | null, elapsed_ms: number, };
export type InputPipelineConfig = { input: InputProbeConfig, timeout_ms: number, };
export type PipelineEnd = "succeeded" | "failed" | "cancelled" | "timed_out";
export type PipelineProbeResult = { sdk_version: string, entry: string, end: PipelineEnd, native_status: string, stop_status: string | null, enqueue_us: number, elapsed_ms: number, release_ms: number, stop_ms: number | null, released: boolean, };
export type InputPipelineResult = { config: InputPipelineConfig, pipeline: PipelineProbeResult, };
export type RunRecord = { id: string, app_id: string, state: RunState, started_at: string, finished_at: string | null, message: string, context?: RunContext, input_probe: InputProbeResult | null, input_pipeline: InputPipelineResult | null, mail: MailProgress | null, map?: MapProgress, coffee?: CoffeeProgress, daily?: Array<DailyProgress>, combat?: CombatProgress, group?: GroupProgress, };
export type RunEvent = { sequence: number, run_id: string | null, at: string, level: string, message: string, };
export type LogRun = { id: string, app_id: string, state: RunState, };
export type LogSession = { id: string, engine_version: string, captured_at: string, event_sequence: number, dropped_events: number, omitted_events: number, truncated_events: number, run: LogRun | null, events: Array<RunEvent>, };
export type LogSessionInfo = { id: string, captured_at: string, event_count: number, };
export type DesktopLogs = { sessions: Array<LogSessionInfo>, selected: LogSession | null, discarded_sessions: number, };
export type LogExport = { path: string, session_id: string, event_count: number, };
export type WindowInfo = { hwnd: string, pid: number, title: string, executable: string, width: number, height: number, keyboard_permission: KeyboardPermission | null, };
export type KeyboardPermission = { engine_integrity: number, game_integrity: number, compatible: boolean, };
export type SdkStatus = { ready: boolean, version: string | null, expected_version: string, binding_version: string, directory: string, message: string, };
export type CaptureMethod = "frame_pool" | "print_window" | "dxgi" | "screen_dc" | "gdi";
export type ScreenshotQuality = { mean_luma: number, dark_pixel_ratio: number, near_black: boolean, };
export type Snapshot = { record_error?: string, engine_version: string, sdk: SdkStatus, window: WindowInfo | null, capture_method: CaptureMethod | null, run: RunRecord | null, events: Array<RunEvent>, event_sequence: number, dropped_events: number, };
export type CacheState = "missing" | "invalid" | "valid";
export type ModelCacheStatus = { id: string, archive: CacheState, files: CacheState, };
export type PreparedModel = { id: string, reused: boolean, downloaded: boolean, };
export type ModelProgress = { id: string, phase: string, completed_bytes: number, total_bytes: number, };
export type ResourceJobKind = "check" | "prepare";
export type ResourceJobState = "running" | "cancelling" | "succeeded" | "failed" | "cancelled";
export type ResourceJob = { id: string, kind: ResourceJobKind, state: ResourceJobState, models: Array<string>, offline: boolean, progress: ModelProgress | null, prepared: Array<PreparedModel>, error: string | null, };
export type ResourceModel = { id: string, name: string, archive_bytes: number, archive: CacheState | null, files: CacheState | null, };
export type ModelResources = { cache_directory: string, models: Array<ResourceModel>, job: ResourceJob | null, };
export type ResourceSelection = { schema_version: number, current: string | null, previous: string | null, can_rollback: boolean, };
export type InstalledResources = { id: string, version: string | null, compatible: boolean, error: string | null, };
export type RuntimeResourceCatalog = { revision: string, selection: ResourceSelection, directory: string, versions: Array<InstalledResources>, };
export type RuntimeResourceRequest = { "type": "discover", sha256: string, } | { "type": "refresh" } | { "type": "download", bytes: number, sha256: string, } | { "type": "install", archive: string, sha256: string, } | { "type": "activate", id: string, revision: string, } | { "type": "rollback", revision: string, };
export type RuntimeResourceJobState = "running" | "cancelling" | "succeeded" | "failed" | "cancelled";
export type RuntimeResourceJob = { id: string, request: RuntimeResourceRequest, state: RuntimeResourceJobState, phase: string, reconnected: boolean, error: string | null, error_code: string | null, progress: RuntimeDownloadProgress | null, downloaded: RuntimeDownloadedArchive | null, };
export type RuntimeDownloadSource = { url: string, bytes: number, sha256: string, };
export type RuntimeDownloadProgress = { phase: string, completed_bytes: number, total_bytes: number, };
export type RuntimeDownloadedArchive = { archive: string, sha256: string, bytes: number, reused: boolean, };
export type DesktopRuntimeResources = { catalog: RuntimeResourceCatalog | null, job: RuntimeResourceJob | null, feed?: RuntimeFeedCatalog, };
export type RuntimeFeedSource = { url: string, sha256: string, };
export type RuntimeRelease = { version: string, engine_version: string, sdk_version: string, binding_version: string, protocol_version: number, resource_schema: number, bytes: number, sha256: string, };
export type RuntimeReleaseOption = { release: RuntimeRelease, compatible: boolean, message: string, };
export type RuntimeFeedCatalog = { id: string, releases: Array<RuntimeReleaseOption>, };
export type Screenshot = { png_base64: string, width: number, height: number, color_order: string, capture_method: CaptureMethod | null, quality: ScreenshotQuality, captured_at: string, elapsed_ms: number, };
export type Command = { "type": "preview_charge_plan", "params": { instance_id: number, group_id: string, revision: number, resources: ChargeResources | null, double_reward_left: number | null, } } | { "type": "combat_scripts" } | { "type": "start_combat", "params": { instance_id: number, revision: number, script_revision: string, allow_input: boolean, confirm_account: boolean, } } | { "type": "preview_combat_script_import", "params": { source: string, } } | { "type": "apply_combat_script_import", "params": { source: string, preview_id: string, } } | { "type": "launch_game", "params": { instance_id: number, revision: number, confirm: boolean, } } | { "type": "model_resources" } | { "type": "start_model_resources", "params": { kind: ResourceJobKind, models: Array<string>, offline: boolean, } } | { "type": "cancel_model_resources", "params": { job_id: string, } } | { "type": "preview_standalone", "params": { instance_id: number, group_id: string, app_id: string, } } | { "type": "start_standalone_trial", "params": { combat_revision?: string, instance_id: number, group_id: string, app_id: string, revision: number, game_day: string, allow_input: boolean, confirm_account: boolean, } } | { "type": "preview_group", "params": { instance_id: number, group_id: string, app_ids?: Array<string>, rerun_completed?: boolean, } } | { "type": "start_group_trial", "params": { combat_revision?: string, instance_id: number, group_id: string, revision: number, game_day: string, app_ids?: Array<string>, rerun_completed?: boolean, allow_input: boolean, confirm_account: boolean, } } | { "type": "run_history", "params": { instance_id: number, } } | { "type": "query_run_history", "params": { instance_id: number, query: HistoryQuery, } } | { "type": "export_run_history", "params": { instance_id: number, query: HistoryQuery, content_id: string, } } | { "type": "retry_run_save" } | { "type": "account_config" } | { "type": "config_backups" } | { "type": "preview_config_backup", "params": { revision: number, id: string, } } | { "type": "restore_config_backup", "params": { revision: number, id: string, preview_id: string, confirm: boolean, } } | { "type": "preview_config_import", "params": { source: string, } } | { "type": "apply_config_import", "params": { source: string, preview_id: string, } } | { "type": "update_account_config", "params": { revision: number, change: ConfigChange, } } | { "type": "start_mail", "params": { config: MailConfig, allow_input: boolean, } } | { "type": "start_map", "params": { config: MapConfig, allow_input: boolean, } } | { "type": "start_coffee", "params": { config: CoffeeConfig, allow_input: boolean, } } | { "type": "start_coffee_from_world", "params": { config: CoffeeConfig, allow_input: boolean, } } | { "type": "pause", "params": { run_id: string, } } | { "type": "resume", "params": { run_id: string, } } | { "type": "snapshot" } | { "type": "applications" } | { "type": "windows" } | { "type": "attach", "params": { hwnd: string, capture_method: CaptureMethod, } } | { "type": "screenshot" } | { "type": "start_probe" } | { "type": "start_input_probe", "params": { config: InputProbeConfig, allow_input: boolean, } } | { "type": "start_input_pipeline", "params": { config: InputPipelineConfig, allow_input: boolean, } } | { "type": "start_application", "params": { app_id: string, } } | { "type": "stop", "params": { run_id: string, } } | { "type": "events", "params": { after: number, } } | { "type": "shutdown" };
export type Request = { version: number, id: string, command: Command, };
export type Payload = { "type": "charge_plan_preview", "data": ChargeGroupPreview } | { "type": "combat_scripts", "data": CombatScriptCatalog } | { "type": "combat_script_import", "data": CombatScriptImportPreview } | { "type": "game_launched", "data": GameLaunchResult } | { "type": "model_resources", "data": ModelResources } | { "type": "group_plan", "data": GroupPlan } | { "type": "run_history", "data": RunHistory } | { "type": "history_page", "data": HistoryPage } | { "type": "history_export", "data": HistoryExport } | { "type": "account_config", "data": AccountConfig } | { "type": "config_backups", "data": ConfigBackups } | { "type": "config_backup_preview", "data": ConfigBackupPreview } | { "type": "config_import_preview", "data": ImportPreview } | { "type": "snapshot", "data": Snapshot } | { "type": "applications", "data": Array<AppDefinition> } | { "type": "windows", "data": Array<WindowInfo> } | { "type": "screenshot", "data": Screenshot } | { "type": "run", "data": RunRecord } | { "type": "events", "data": Array<RunEvent> } | { "type": "ack" };
export type ApiError = { code: string, message: string, details: string | null, };
export type Response = { version: number, id: string, result: Payload | null, error: ApiError | null, };
export type DesktopUpdate = { "type": "run_notifications", "data": RunNotifications } | { "type": "open_run_notifications" } | { "type": "after_done", "data": AfterDoneStatus | null } | { "type": "snapshot", "data": Snapshot } | { "type": "offline", "data": ApiError } | { "type": "privilege", "data": EnginePrivilege } | { "type": "shortcuts", "data": ShortcutStatus };
export type EnginePrivilege = { engine_integrity: number, desktop_integrity: number, };
export const generalSettingsFields = [
  {
    "key": "capture_method",
    "kind": "select",
    "label": "默认截图方式",
    "options": [
      {
        "label": "FramePool",
        "value": "frame_pool"
      },
      {
        "label": "PrintWindow",
        "value": "print_window"
      },
      {
        "label": "DXGI",
        "value": "dxgi"
      },
      {
        "label": "ScreenDC",
        "value": "screen_dc"
      },
      {
        "label": "GDI",
        "value": "gdi"
      }
    ]
  },
  {
    "key": "timeout_ms",
    "kind": "integer",
    "label": "默认邮件超时（ms）",
    "max": 60000,
    "min": 1
  },
  {
    "key": "start_delay_ms",
    "kind": "integer",
    "label": "默认启动 / 继续延时（ms）",
    "max": 10000,
    "min": 0
  }
] as const;
export const gameRegionOptions = [
  {
    "label": "国服",
    "value": "cn"
  },
  {
    "label": "B服",
    "value": "cn_b"
  },
  {
    "label": "美服",
    "value": "us"
  },
  {
    "label": "欧服",
    "value": "eu"
  },
  {
    "label": "亚服",
    "value": "asia"
  },
  {
    "label": "港澳台服",
    "value": "twhkmo"
  }
] as const;
export const runPolicyOptions = [
  {
    "label": "每次运行",
    "value": "always"
  },
  {
    "label": "跳过本游戏日已完成",
    "value": "skip_daily_success"
  }
] as const;
export const shortcutKeys = [
  "f1",
  "f2",
  "f3",
  "f4",
  "f5",
  "f6",
  "f7",
  "f8",
  "f9",
  "f10",
  "f11"
] as const;
export const coffeeSettingsFields = [
  {
    "key": "kind",
    "kind": "select",
    "label": "默认咖啡",
    "options": [
      {
        "label": "汀曼特调",
        "value": "tinman"
      },
      {
        "label": "浓缩咖啡",
        "value": "espresso"
      }
    ]
  },
  {
    "key": "timeout_ms",
    "kind": "integer",
    "label": "默认咖啡超时（ms）",
    "max": 60000,
    "min": 1
  },
  {
    "key": "start_delay_ms",
    "kind": "integer",
    "label": "默认咖啡启动 / 继续延时（ms）",
    "max": 10000,
    "min": 0
  }
] as const;
export const coffeeDefaultConfig = {
  "kind": "tinman",
  "timeout_ms": 60000,
  "start_delay_ms": 3000
} as const;
export const dailySettingsFields = [
  {
    "key": "timeout_ms",
    "label": "日常超时（ms）",
    "max": 180000,
    "min": 1
  },
  {
    "key": "start_delay_ms",
    "label": "日常启动 / 继续延时（ms）",
    "max": 10000,
    "min": 0
  }
] as const;
export const dailyDefaultConfig = {
  "timeout_ms": 120000,
  "start_delay_ms": 3000
} as const;
export const suibianTempleDefaults = {
  "auto_manage_enabled": true,
  "yum_cha_sin": true,
  "yum_cha_sin_period_refresh": true,
  "adventure_duration": "hour_20",
  "adventure_missions": [
    "RESEARCH_3_4",
    "RESEARCH_2_4",
    "RESEARCH_1_4",
    "COMMUNITY_3_4"
  ],
  "craft_drag_times": 10,
  "good_goods_purchase_enabled": false,
  "boo_box_purchase_enabled": false,
  "boo_box_adventure_price": "s4",
  "boo_box_craft_price": "s4",
  "boo_box_sell_price": "s4",
  "legacy_pawnshop": {
    "omnicoin_enabled": true,
    "omnicoin_priority": [
      "hifi_master_copy",
      "senior_investigator_log",
      "w_engine_energy_module",
      "ether_plating_agent",
      "prepaid_power_card"
    ],
    "crest_enabled": true,
    "crest_priority": [
      "bangboo_system_widget",
      "denny"
    ],
    "crest_unlimited_denny_enabled": false
  },
  "timeout_ms": 900000
} as SuibianTempleConfig;
export const suibianDurationOptions = [{"label":"3分钟","value":"min_3"},{"label":"15分钟","value":"min_15"},{"label":"1小时","value":"hour_1"},{"label":"2小时","value":"hour_2"},{"label":"6小时","value":"hour_6"},{"label":"12小时","value":"hour_12"},{"label":"20小时","value":"hour_20"}] as const;
export const suibianMissionOptions = [{"label":"制造区1-1","value":"CRAFT_1_1"},{"label":"制造区1-2","value":"CRAFT_1_2"},{"label":"制造区1-3","value":"CRAFT_1_3"},{"label":"制造区1-4","value":"CRAFT_1_4"},{"label":"制造区2-1","value":"CRAFT_2_1"},{"label":"制造区2-2","value":"CRAFT_2_2"},{"label":"制造区2-3","value":"CRAFT_2_3"},{"label":"制造区2-4","value":"CRAFT_2_4"},{"label":"制造区3-1","value":"CRAFT_3_1"},{"label":"制造区3-2","value":"CRAFT_3_2"},{"label":"制造区3-3","value":"CRAFT_3_3"},{"label":"制造区3-4","value":"CRAFT_3_4"},{"label":"社区旧址1-1","value":"COMMUNITY_1_1"},{"label":"社区旧址1-2","value":"COMMUNITY_1_2"},{"label":"社区旧址1-3","value":"COMMUNITY_1_3"},{"label":"社区旧址1-4","value":"COMMUNITY_1_4"},{"label":"社区旧址2-1","value":"COMMUNITY_2_1"},{"label":"社区旧址2-2","value":"COMMUNITY_2_2"},{"label":"社区旧址2-3","value":"COMMUNITY_2_3"},{"label":"社区旧址2-4","value":"COMMUNITY_2_4"},{"label":"社区旧址3-1","value":"COMMUNITY_3_1"},{"label":"社区旧址3-2","value":"COMMUNITY_3_2"},{"label":"社区旧址3-3","value":"COMMUNITY_3_3"},{"label":"社区旧址3-4","value":"COMMUNITY_3_4"},{"label":"科研院旧址1-1","value":"RESEARCH_1_1"},{"label":"科研院旧址1-2","value":"RESEARCH_1_2"},{"label":"科研院旧址1-3","value":"RESEARCH_1_3"},{"label":"科研院旧址1-4","value":"RESEARCH_1_4"},{"label":"科研院旧址2-1","value":"RESEARCH_2_1"},{"label":"科研院旧址2-2","value":"RESEARCH_2_2"},{"label":"科研院旧址2-3","value":"RESEARCH_2_3"},{"label":"科研院旧址2-4","value":"RESEARCH_2_4"},{"label":"科研院旧址3-1","value":"RESEARCH_3_1"},{"label":"科研院旧址3-2","value":"RESEARCH_3_2"},{"label":"科研院旧址3-3","value":"RESEARCH_3_3"},{"label":"科研院旧址3-4","value":"RESEARCH_3_4"}] as const;
export const suibianPriceOptions = [{"label":"40000 及以上","value":"s1"},{"label":"35000 及以上","value":"s2"},{"label":"30000 及以上","value":"s3"},{"label":"25000 及以上","value":"s4"},{"label":"不购买","value":"none"}] as const;
export const afterDoneOptions = [
  {
    "label": "无",
    "value": "none"
  },
  {
    "label": "关闭游戏",
    "value": "close_game"
  },
  {
    "label": "关机",
    "value": "shutdown"
  },
  {
    "label": "休眠",
    "value": "hibernate"
  },
  {
    "label": "睡眠",
    "value": "sleep"
  }
] as const;
export const gameLaunchDefaultConfig = {
  "executable": "",
  "use_arguments": false,
  "fullscreen": false,
  "borderless": false,
  "monitor": 1
} as const;
export const runNotificationOptions = [
  {
    "label": "关闭",
    "value": "off"
  },
  {
    "label": "仅异常",
    "value": "errors"
  },
  {
    "label": "全部结果",
    "value": "all"
  }
] as const;
export const screenSampleKindOptions = [
  {
    "label": "活跃度 · 日常页",
    "value": "engagement_daily"
  },
  {
    "label": "活跃度 · 领取确认",
    "value": "engagement_confirm"
  },
  {
    "label": "活跃度 · 奖励预览",
    "value": "engagement_preview"
  },
  {
    "label": "咖啡 · 点单选项",
    "value": "coffee_options"
  },
  {
    "label": "咖啡 · 电量奖励",
    "value": "coffee_energy"
  },
  {
    "label": "咖啡 · 增益确认",
    "value": "coffee_buff"
  },
  {
    "label": "其他画面",
    "value": "other"
  }
] as const;
export const redemptionSampleCodes = {
  "PHOENIX1021": "2026-10-10",
  "ZZZ888": "2099-01-01"
} as const;
