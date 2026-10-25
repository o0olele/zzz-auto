//! 导航线在战斗区域内隐藏时，按当前小地图道路连通关系接近目标。
use crate::{map_exploration::map_icons, navigation_vision};
use image::{GrayImage, Luma, RgbImage};
use od_core::{map_exploration::ExplorationTarget, world_patrol::PatrolPoint};
use std::{cmp::Reverse, collections::BinaryHeap};

const SIZE: usize = 210;
const ORIGIN: PatrolPoint = PatrolPoint { x: 105, y: 105 };

#[derive(Debug, Clone, serde::Serialize)]
pub struct RoadRoute {
    pub target: PatrolPoint,
    pub waypoint: PatrolPoint,
    pub remaining: f64,
}

pub fn local_road_route(mini: &RgbImage, target: ExplorationTarget) -> Option<RoadRoute> {
    let icons: Vec<_> = map_icons(mini, target)
        .into_iter()
        .filter(|icon| {
            ORIGIN.distance(PatrolPoint {
                x: icon.center[0],
                y: icon.center[1],
            }) < 94.0
        })
        .collect();
    let [icon] = icons.as_slice() else {
        return None;
    };
    road_route_to(
        mini,
        PatrolPoint {
            x: icon.center[0],
            y: icon.center[1],
        },
    )
}

fn clear_line(road: &GrayImage, to: PatrolPoint) -> bool {
    let dx = to.x - ORIGIN.x;
    let dy = to.y - ORIGIN.y;
    let steps = dx.abs().max(dy.abs()) * 2;
    let mut previous = ORIGIN;
    for step in 0..=steps {
        let x = ORIGIN.x + (f64::from(dx * step) / f64::from(steps.max(1))).round() as i32;
        let y = ORIGIN.y + (f64::from(dy * step) / f64::from(steps.max(1))).round() as i32;
        if road.get_pixel(x as u32, y as u32)[0] == 0 {
            return false;
        }
        if x != previous.x
            && y != previous.y
            && (road.get_pixel(x as u32, previous.y as u32)[0] == 0
                || road.get_pixel(previous.x as u32, y as u32)[0] == 0)
        {
            return false;
        }
        previous = PatrolPoint { x, y };
    }
    true
}

/// 仅在目标可见、玩家有效且道路连通时提供局部路点，不跨地图边界或盲走。
pub fn road_route_to(mini: &RgbImage, target: PatrolPoint) -> Option<RoadRoute> {
    if mini.dimensions() != (210, 210)
        || !navigation_vision::player_found(mini)
        || !(24.0..94.0).contains(&ORIGIN.distance(target))
    {
        return None;
    }
    let mut road = GrayImage::new(210, 210);
    let mut boundary = GrayImage::new(210, 210);
    for (x, y, pixel) in mini.enumerate_pixels() {
        if (x as i32 - 105).pow(2) + (y as i32 - 105).pow(2) > 92 * 92
            || (x as i32 - 207).pow(2) + (y as i32 - 189).pow(2) < 50 * 50
        {
            continue;
        }
        let [r, g, b] = pixel.0.map(i32::from);
        let min = r.min(g).min(b);
        let max = r.max(g).max(b);
        let luma = (r * 4899 + g * 9617 + b * 1868 + 8192) >> 14;
        let u = (((b - luma) * 8061 + 8192) >> 14) + 128;
        let v = (((r - luma) * 14369 + 8192) >> 14) + 128;
        let view = (100..=110).contains(&u) || (140..=150).contains(&v);
        // 暗色斜纹为道路；视野光锥染黄时蓝通道仍可约束道路底色。
        let tinted_road = view && b <= 45 && max <= 150;
        if max >= 75 && !tinted_road {
            boundary.put_pixel(x, y, Luma([255]));
        }
        if (max <= 62 && max - min <= 12) || tinted_road {
            road.put_pixel(x, y, Luma([255]));
        }
    }
    road = navigation_vision::morphology(&navigation_vision::morphology(&road, true), false);
    // 填斜纹间隙时不能抹掉浅色墙线或平台边界。
    for (x, y, pixel) in boundary.enumerate_pixels() {
        if pixel[0] != 0 {
            road.put_pixel(x, y, Luma([0]));
        }
    }
    // 箭头的灰白描边会遮住脚下道路；只补回已验证玩家图标的局部遮挡。
    let mut player = navigation_vision::player_mask(mini);
    for _ in 0..5 {
        player = navigation_vision::morphology(&player, true);
    }
    for (x, y, pixel) in player.enumerate_pixels() {
        if pixel[0] != 0 {
            road.put_pixel(x, y, Luma([255]));
        }
    }
    // 给墙线和平台边缘留出余量；对角移动也不能切过拐角。
    road = navigation_vision::morphology(&navigation_vision::morphology(&road, false), false);
    let start = 105 * SIZE + 105;
    if road.get_pixel(105, 105)[0] == 0 {
        return None;
    }
    let mut distance = vec![u32::MAX; SIZE * SIZE];
    let mut parent = vec![usize::MAX; SIZE * SIZE];
    let mut queue = BinaryHeap::from([Reverse((0u32, start))]);
    distance[start] = 0;
    let mut goal = None;
    while let Some(Reverse((cost, at))) = queue.pop() {
        if cost != distance[at] {
            continue;
        }
        let (x, y) = ((at % SIZE) as i32, (at / SIZE) as i32);
        if (PatrolPoint { x, y }).distance(target) <= 16.0 {
            goal = Some(at);
            break;
        }
        for (dx, dy) in [
            (-1, 0),
            (1, 0),
            (0, -1),
            (0, 1),
            (-1, -1),
            (-1, 1),
            (1, -1),
            (1, 1),
        ] {
            let (nx, ny) = (x + dx, y + dy);
            if !(0..210).contains(&nx)
                || !(0..210).contains(&ny)
                || road.get_pixel(nx as u32, ny as u32)[0] == 0
                || (dx != 0
                    && dy != 0
                    && (road.get_pixel(nx as u32, y as u32)[0] == 0
                        || road.get_pixel(x as u32, ny as u32)[0] == 0))
            {
                continue;
            }
            let next = ny as usize * SIZE + nx as usize;
            let next_cost = cost + if dx == 0 || dy == 0 { 10 } else { 14 };
            if next_cost < distance[next] {
                distance[next] = next_cost;
                parent[next] = at;
                queue.push(Reverse((next_cost, next)));
            }
        }
    }
    let goal = goal?;
    let remaining = f64::from(distance[goal]) / 10.0;
    let mut path = vec![];
    let mut at = goal;
    while at != start {
        path.push(at);
        at = parent[at];
    }
    let mut waypoint = None;
    for at in path.into_iter().rev() {
        if distance[at] > 350 {
            break;
        }
        let point = PatrolPoint {
            x: (at % SIZE) as i32,
            y: (at / SIZE) as i32,
        };
        if !clear_line(&road, point) {
            break;
        }
        if ORIGIN.distance(point) >= 6.0 {
            waypoint = Some(point);
        }
    }
    Some(RoadRoute {
        target,
        waypoint: waypoint?,
        remaining,
    })
}
