use crate::{InputProbeAction, InputSide, MouseButton};
use std::collections::BTreeMap;

/// 逻辑按键可共享物理按键；摇杆方向相加，松开一个方向不影响其他方向。
#[derive(Default)]
pub struct InputLedger { held: BTreeMap<String, Vec<InputProbeAction>>, gamepad: bool }
impl InputLedger {
    pub fn new(gamepad: bool) -> Self { Self { gamepad, ..Self::default() } }
    pub fn clear(&mut self) { self.held.clear(); }
    pub fn set(&mut self, name: &str, actions: Vec<InputProbeAction>, pressed: bool) -> Vec<(InputProbeAction, bool)> {
        let before = self.physical();
        if pressed { self.held.insert(name.into(), actions); } else { self.held.remove(name); }
        let after = self.physical();
        let mut effects = Vec::new();
        for (slot, action) in &before { if !after.contains_key(slot) { effects.push((*action, false)); } }
        for (slot, action) in after { if before.get(&slot) != Some(&action) { effects.push((action, true)); } }
        effects
    }
    fn physical(&self) -> BTreeMap<i32, InputProbeAction> {
        let mut result = BTreeMap::new();
        let mut sticks = [[0i64; 2]; 2];
        let mut triggers = [0; 2];
        for actions in self.held.values() { for action in actions {
            match *action {
                InputProbeAction::Key { key } if self.gamepad => { for bit in 0..19 { if key & (1 << bit) != 0 { result.insert(1 << bit, InputProbeAction::Key { key: 1 << bit }); } } },
                InputProbeAction::Key { key } => { result.insert(key, *action); },
                InputProbeAction::MouseButton { button, .. } => { result.insert(300 + match button { MouseButton::Left => 0, MouseButton::Right => 1, MouseButton::Middle => 2, MouseButton::X1 => 3, MouseButton::X2 => 4 }, *action); },
                InputProbeAction::Stick { stick, x, y } => { let index = usize::from(stick == InputSide::Right); sticks[index][0] += x as i64; sticks[index][1] += y as i64; },
                InputProbeAction::Trigger { trigger, value } => { let index = usize::from(trigger == InputSide::Right); triggers[index] = triggers[index].max(value); },
            }
        } }
        for i in 0..2 {
            let side = if i == 0 { InputSide::Left } else { InputSide::Right };
            let x = sticks[i][0].clamp(-32768, 32767) as i32; let y = sticks[i][1].clamp(-32768, 32767) as i32;
            if x != 0 || y != 0 { result.insert(1_000_000 + i as i32, InputProbeAction::Stick { stick: side, x, y }); }
            if triggers[i] != 0 { result.insert(1_000_002 + i as i32, InputProbeAction::Trigger { trigger: side, value: triggers[i] }); }
        }
        result
    }
}
