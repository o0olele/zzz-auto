use anyhow::{Result,ensure};
use crate::{combat::CombatConfig,InputMode,InputProbeAction,InputSide,MouseButton};
use serde_json::Value;

pub fn settings(battle:&Value,game:&Value)->Result<(CombatConfig,Vec<String>)> {
    let mut settings=CombatConfig::default();let mut warnings=Vec::new();
    let string=|map:&Value,key:&str|map[key].as_str().map(str::to_string);
    if let Some(name)=string(battle,"auto_battle_config") {settings.script=name;}
    if let Some(name)=string(battle,"dodge_assistant_config") {settings.dodge_script=name;}
    settings.use_merged=battle["use_merged_file"].as_bool().unwrap_or(true);
    settings.auto_ultimate=battle["auto_ultimate_enabled"].as_bool().unwrap_or(false);
    if let Some(interval)=battle["screenshot_interval"].as_f64() {ensure!(interval.is_finite()&&(0.001..=1.0).contains(&interval),"旧截图间隔超出 0.001..1 秒");settings.frame_interval_ms=(interval*1000.0).round() as u32;}
    let old_mode=battle["control_method"].as_str().unwrap_or("keyboard");
    let background=battle["background_mode"].as_bool().unwrap_or(["xbox","ds4"].contains(&old_mode));
    settings.mode=if background {if game["background_gamepad_type"].as_str().unwrap_or(if old_mode=="ds4"{"ds4"}else{"xbox"})=="ds4" {InputMode::Ds4}else{InputMode::Xbox}}else{InputMode::Foreground};
    let prefix=match settings.mode {InputMode::Xbox=>"xbox_key",InputMode::Ds4=>"ds4_key",_=>"key"};
    if settings.mode.is_gamepad()&&let Some(seconds)=game[format!("{prefix}_press_time")].as_f64() {ensure!(seconds.is_finite()&&(0.001..=1.0).contains(&seconds),"旧手柄点按时间超出范围");settings.tap_ms=(seconds*1000.0).round() as u32;}
    for (name,key) in [("闪避","dodge"),("切换角色-下一个","switch_next"),("切换角色-上一个","switch_prev"),("切换后援","switch_backup"),("普通攻击","normal_attack"),("特殊攻击","special_attack"),("终结技","ultimate"),("连携技-左","chain_left"),("连携技-右","chain_right"),("连携技-取消","chain_cancel"),("移动-前","move_w"),("移动-后","move_s"),("移动-左","move_a"),("移动-右","move_d"),("锁定敌人","lock")] {
        if let Some(value)=game[format!("{prefix}_{key}")].as_str() {
            match keys(value,settings.mode) {Ok(actions)=>{settings.bindings.insert(name.into(),actions);},Err(_)=>warnings.push(format!("旧按键 {prefix}_{key} 无法转换，采用默认值；原值已归档，请核对。"))}
        }
    }
    settings.validate().map_err(anyhow::Error::msg)?;
    Ok((settings,warnings))
}
pub fn keys(value:&str,mode:InputMode)->Result<Vec<InputProbeAction>> {
    let mut actions=Vec::new();
    for key in value.split('+') {
        let mut key=key.trim().to_lowercase();
        if let Some(index)=key.strip_prefix("xbox_").or_else(||key.strip_prefix("ds4_")).and_then(|s|s.parse::<usize>().ok()) {
            let tokens=if mode==InputMode::Ds4 {&["ds4_cross","ds4_circle","ds4_square","ds4_triangle","ds4_l2","ds4_r2","ds4_l1","ds4_r1","ds4_ls_up","ds4_ls_down","ds4_ls_left","ds4_ls_right","ds4_l_thumb","ds4_r_thumb"]}else{&["xbox_a","xbox_b","xbox_x","xbox_y","xbox_lt","xbox_rt","xbox_lb","xbox_rb","xbox_ls_up","xbox_ls_down","xbox_ls_left","xbox_ls_right","xbox_l_thumb","xbox_r_thumb"]};
            key=tokens.get(index).ok_or_else(||anyhow::anyhow!("未知旧手柄按键"))?.to_string();
        }
        let action=if mode.is_gamepad() {
            let token=key.strip_prefix(if mode==InputMode::Ds4{"ds4_"}else{"xbox_"}).ok_or_else(||anyhow::anyhow!("手柄按键前缀不匹配"))?;
            match token {
                "lt"|"l2"=>InputProbeAction::Trigger{trigger:InputSide::Left,value:255},"rt"|"r2"=>InputProbeAction::Trigger{trigger:InputSide::Right,value:255},
                "ls_up"|"ls_down"|"ls_left"|"ls_right"=>{let(x,y)=match token{"ls_up"=>(0,32767),"ls_down"=>(0,-32768),"ls_left"=>(-32768,0),_=>(32767,0)};InputProbeAction::Stick{stick:InputSide::Left,x,y}},
                _=>{let key=match token{"a"|"cross"=>4096,"b"|"circle"=>8192,"x"|"square"=>16384,"y"|"triangle"=>32768,"lb"|"l1"=>256,"rb"|"r1"=>512,"l_thumb"=>64,"r_thumb"=>128,"start"|"options"=>16,"back"|"share"=>32,"dpad_up"=>1,"dpad_down"=>2,"dpad_left"=>4,"dpad_right"=>8,"ps" if mode==InputMode::Ds4=>0x10000,"touchpad" if mode==InputMode::Ds4=>0x20000,_=>anyhow::bail!("未知手柄按键")};InputProbeAction::Key{key}}
            }
        } else if let Some(button)=key.strip_prefix("mouse_") {
            let button=match button{"left"=>MouseButton::Left,"right"=>MouseButton::Right,"middle"=>MouseButton::Middle,"x1"=>MouseButton::X1,"x2"=>MouseButton::X2,_=>anyhow::bail!("未知鼠标按键")};InputProbeAction::MouseButton{button,x:960,y:540}
        } else {
            let vk=if key.len()==1&&key.as_bytes()[0].is_ascii_alphanumeric(){key.as_bytes()[0].to_ascii_uppercase() as i32}else if let Some(n)=key.strip_prefix('f').and_then(|n|n.parse::<i32>().ok()).filter(|n|(1..=24).contains(n)){111+n}else{match key.as_str(){"shift"=>16,"ctrl"|"control"=>17,"alt"=>18,"space"=>32,"tab"=>9,"enter"=>13,"esc"|"escape"=>27,_=>anyhow::bail!("未知键盘按键")}};InputProbeAction::Key{key:vk}
        };
        if !actions.contains(&action){actions.push(action);}
    }
    ensure!(!actions.is_empty()&&actions.len()<=8,"按键组合数量无效");
    for action in &actions {crate::InputProbeConfig{mode,action:*action,hold_ms:1,start_delay_ms:0}.validate().map_err(anyhow::Error::msg)?;}
    Ok(actions)
}
