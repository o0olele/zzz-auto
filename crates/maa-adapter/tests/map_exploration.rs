use image::{Rgb, RgbImage};
use maa_adapter::map_exploration::{guide_point, local_road_route, map_icons, road_route_to};
use od_core::map_exploration::ExplorationTarget;
use od_core::world_patrol::PatrolPoint;

#[test]
fn bidirectional_bugged_line_does_not_flip_the_steering_target() {
    let world = image::load_from_memory(include_bytes!(
        "fixtures/map-exploration/minimap-ambiguous-route.png"
    ))
    .unwrap()
    .to_rgb8();
    let mini = image::imageops::crop_imm(&world, 1534, 201, 210, 210).to_image();
    let routes = maa_adapter::map_exploration::guide_routes(&mini);
    assert_eq!(
        routes.len(),
        2,
        "当前异常小地图中应同时识别到左右两条路径：{routes:?}"
    );
    assert!(guide_point(&mini).is_none(), "不能任取左右一条异常绿线");
    assert_eq!(map_icons(&mini, ExplorationTarget::Observation).len(), 1);
    assert!(
        local_road_route(&mini, ExplorationTarget::Observation).is_none(),
        "当前画面无法确认道路连通，不能跨过被遮挡的边界"
    );
    let marker = maa_adapter::map_exploration::observation_world_marker(&world, [546, 455])
        .expect("应核对距离旁的观测奇点世界标记");
    assert!(
        (marker[0] - 487).abs() < 5 && (marker[1] - 448).abs() < 5,
        "{marker:?}"
    );
    assert!(
        maa_adapter::map_exploration::observation_world_marker(&world, [1050, 455]).is_none(),
        "任意距离附近的背景不能作为观测奇点标记"
    );
    let centered = image::load_from_memory(include_bytes!(
        "fixtures/map-exploration/minimap-ambiguous-centered.png"
    ))
    .unwrap()
    .to_rgb8();
    let marker = maa_adapter::map_exploration::observation_world_marker(&centered, [987, 445])
        .expect("圆环并入 OCR 文本框时也应核对标记");
    assert!((marker[0] - 959).abs() < 5, "{marker:?}");
    let mini = image::imageops::crop_imm(&centered, 1534, 201, 210, 210).to_image();
    assert!(
        guide_point(&mini).is_some(),
        "回放中一侧异常绿线暂时消失，仍须保持本目标的异常导航策略"
    );
}

#[test]
fn bottom_action_slot_f_is_detected_after_character_faces_the_target() {
    for (bytes, expected) in [
        (
            include_bytes!("fixtures/map-exploration/observation-arrival-f-ready.png").as_slice(),
            true,
        ),
        (
            include_bytes!("fixtures/map-exploration/observation-arrival-facing.png").as_slice(),
            false,
        ),
        (
            include_bytes!("fixtures/map-exploration/observation-arrival-edge.png").as_slice(),
            false,
        ),
        (
            include_bytes!("fixtures/map-exploration/follow-world.png").as_slice(),
            false,
        ),
    ] {
        let image = image::load_from_memory(bytes).unwrap().to_rgb8();
        assert_eq!(
            maa_adapter::map_exploration::interaction_f_key(&image),
            expected
        );
    }
}

#[test]
fn battle_exit_minimap_keeps_player_heading_when_tracking_dots_are_missing() {
    let world = image::load_from_memory(include_bytes!(
        "fixtures/map-exploration/post-battle-world.png"
    ))
    .unwrap()
    .to_rgb8();
    let mini = image::imageops::crop_imm(&world, 1534, 201, 210, 210).to_image();
    assert!(
        maa_adapter::navigation_vision::heading(&mini)
            .unwrap()
            .is_some()
    );
    assert!(
        guide_point(&mini).is_none(),
        "这张战后画面的引导点确实消失了"
    );
}

fn battle_mini() -> RgbImage {
    let world = image::load_from_memory(include_bytes!(
        "fixtures/map-exploration/post-battle-world.png"
    ))
    .unwrap()
    .to_rgb8();
    image::imageops::crop_imm(&world, 1534, 201, 210, 210).to_image()
}

#[test]
fn battle_zone_without_dots_uses_the_unique_target_and_connected_road() {
    let mini = battle_mini();
    let icons = map_icons(&mini, ExplorationTarget::Observation);
    assert_eq!(icons.len(), 1, "{icons:?}");
    let route = local_road_route(&mini, ExplorationTarget::Observation)
        .expect("战后无绿点也应能沿道路离开");
    assert!(
        (route.target.x - 27).abs() < 8 && (route.target.y - 70).abs() < 8,
        "{route:?}"
    );
    assert!(route.waypoint.x < 90 && route.waypoint.y < 110, "{route:?}");
    assert!(route.remaining > 30.0);
    assert!(local_road_route(&mini, ExplorationTarget::Cargo).is_none());
}

fn corner_road() -> RgbImage {
    let mut mini = RgbImage::from_pixel(210, 210, Rgb([100, 120, 140]));
    for y in 95..116 {
        for x in 95..166 {
            mini.put_pixel(x, y, Rgb([40, 40, 40]));
        }
    }
    for y in 45..116 {
        for x in 145..166 {
            mini.put_pixel(x, y, Rgb([40, 40, 40]));
        }
    }
    let original = battle_mini();
    for y in 92..119 {
        for x in 92..119 {
            let pixel = *original.get_pixel(x, y);
            let [h, s, v] = maa_adapter::combat_vision::hsv(pixel.0);
            if (14..=34).contains(&h) && s >= 90 && v >= 175 {
                mini.put_pixel(x, y, pixel);
            }
        }
    }
    mini
}

#[test]
fn local_road_path_takes_the_corner_and_rejects_disconnected_or_narrow_paths() {
    let target = PatrolPoint { x: 155, y: 35 };
    let mut mini = corner_road();
    let route = road_route_to(&mini, target).expect("L 形道路应有连通路径");
    assert!(
        route.waypoint.x >= 125 && route.waypoint.y >= 99,
        "须先向右走，不能直穿拐角：{route:?}"
    );
    for y in 70..90 {
        for x in 145..166 {
            mini.put_pixel(x, y, Rgb([100, 120, 140]));
        }
    }
    assert!(road_route_to(&mini, target).is_none(), "不能跨过断开的道路");
    let mut wall = corner_road();
    for y in 90..122 {
        wall.put_pixel(130, y, Rgb([110, 110, 110]));
    }
    assert!(
        road_route_to(&wall, target).is_none(),
        "道路闭合处理不能抹掉一像素墙线"
    );
    let mut narrow = corner_road();
    for y in 95..116 {
        if !(104..107).contains(&y) {
            for x in 120..145 {
                narrow.put_pixel(x, y, Rgb([100, 120, 140]));
            }
        }
    }
    assert!(
        road_route_to(&narrow, target).is_none(),
        "不能把不足通行余量的细线当成道路"
    );
    assert!(
        road_route_to(&corner_road(), PatrolPoint { x: 108, y: 105 }).is_none(),
        "不能在目标图标下盲走"
    );
}

#[test]
fn local_road_navigation_rejects_missing_or_ambiguous_targets() {
    assert!(local_road_route(&corner_road(), ExplorationTarget::Observation).is_none());
    let mut mini = battle_mini();
    let icon = image::imageops::crop_imm(&mini, 8, 56, 38, 30).to_image();
    image::imageops::overlay(&mut mini, &icon, 130, 50);
    assert!(local_road_route(&mini, ExplorationTarget::Observation).is_none());
}

#[test]
fn supplied_map_does_not_mistake_supply_or_combat_for_cargo() {
    let rgb = image::load_from_memory(include_bytes!("fixtures/map-exploration/map.png"))
        .unwrap()
        .to_rgb8();
    let observation = map_icons(&rgb, ExplorationTarget::Observation);
    let cargo = map_icons(&rgb, ExplorationTarget::Cargo);
    assert_eq!(observation.len(), 1, "{observation:?}");
    assert!(cargo.is_empty(), "三个金色方块不是小卡格车：{cargo:?}");
    assert!(
        observation
            .iter()
            .any(|p| (p.center[0] - 493).abs() < 15 && (p.center[1] - 272).abs() < 15),
        "{observation:?}"
    );
    assert!(
        cargo
            .iter()
            .chain(&observation)
            .all(|p| (p.center[0] - 975).abs() > 40 && (p.center[0] - 1150).abs() > 40)
    );
}

#[test]
fn minimum_zoom_frame_has_dim_observation_and_cargo() {
    let rgb = image::load_from_memory(include_bytes!("fixtures/map-exploration/minimum-map.png"))
        .unwrap()
        .to_rgb8();
    let eyes = map_icons(&rgb, ExplorationTarget::Observation);
    let cargo = map_icons(&rgb, ExplorationTarget::Cargo);
    assert!(
        eyes.iter()
            .any(|icon| (icon.center[0] - 840).abs() < 20 && (icon.center[1] - 445).abs() < 20),
        "{eyes:?}"
    );
    assert!(
        cargo.is_empty(),
        "不可把 01/02 的金色方块当作卡格车：{cargo:?}"
    );
    let zoom = maa_adapter::map_exploration::zoom_control(&rgb).expect("应识别最小缩放滑块");
    assert!(zoom.at_minimum, "{zoom:?}");
    assert!((zoom.knob[1] - 1020).abs() < 5, "{zoom:?}");
}

#[test]
fn cargo_shape_and_zoom_are_recognized_in_new_viewport() {
    let mut rgb =
        image::load_from_memory(include_bytes!("fixtures/map-exploration/panned-map.png"))
            .unwrap()
            .to_rgb8();
    let cargo = image::load_from_memory(include_bytes!(
        "../../../resources/source/map-exploration/cargo.png"
    ))
    .unwrap()
    .to_rgb8();
    image::imageops::replace(&mut rgb, &cargo, 500, 450);
    let icons = map_icons(&rgb, ExplorationTarget::Cargo);
    assert!(
        icons
            .iter()
            .any(|p| (p.center[0] - 517).abs() < 8 && (p.center[1] - 468).abs() < 8),
        "{icons:?}"
    );
    let zoomed = image::load_from_memory(include_bytes!("fixtures/map-exploration/zoomed-map.png"))
        .unwrap()
        .to_rgb8();
    let zoom = maa_adapter::map_exploration::zoom_control(&zoomed).unwrap();
    assert!(!zoom.at_minimum, "{zoom:?}");
    assert!((zoom.knob[0] - 1020).abs() < 5, "{zoom:?}");
}

#[test]
fn actual_panning_does_not_reject_an_eye_with_changed_road_background() {
    for (bytes, center) in [
        (
            include_bytes!("fixtures/map-exploration/live-panned-0.png").as_slice(),
            [938, 444],
        ),
        (
            include_bytes!("fixtures/map-exploration/live-panned-1.png").as_slice(),
            [940, 238],
        ),
    ] {
        let rgb = image::load_from_memory(bytes).unwrap().to_rgb8();
        let at = std::time::Instant::now();
        let eyes = map_icons(&rgb, ExplorationTarget::Observation);
        eprintln!("实际拖图识别耗时 {:?}", at.elapsed());
        assert!(
            eyes.iter()
                .any(|p| (p.center[0] - center[0]).abs() < 15
                    && (p.center[1] - center[1]).abs() < 15),
            "{eyes:?}"
        );
        assert_eq!(eyes.len(), 1, "{eyes:?}");
    }
    let empty = image::load_from_memory(include_bytes!(
        "fixtures/map-exploration/reset-minimum-map.png"
    ))
    .unwrap()
    .to_rgb8();
    assert!(map_icons(&empty, ExplorationTarget::Observation).is_empty());
}

#[test]
fn supplied_minimap_follows_nearby_dots() {
    let rgb = image::load_from_memory(include_bytes!("fixtures/map-exploration/mini-map.png"))
        .unwrap()
        .to_rgb8();
    let guide = guide_point(&rgb).expect("截图应有绿色引导点");
    assert!(
        maa_adapter::navigation_vision::heading(&rgb)
            .unwrap()
            .is_some(),
        "截图应可识别玩家朝向"
    );
    assert!(
        (125..=158).contains(&guide.x) && (96..=116).contains(&guide.y),
        "{guide:?}"
    );
}

#[test]
fn actual_follow_line_remains_valid_when_nearest_dot_is_tinted_by_heading_cone() {
    let rgb = image::load_from_memory(include_bytes!("fixtures/map-exploration/follow-mini.png"))
        .unwrap()
        .to_rgb8();
    let guide = guide_point(&rgb).expect("最近一颗点变黄后，仍应连接后续的两颗绿点");
    assert!(
        (50..=80).contains(&guide.x) && (100..=120).contains(&guide.y),
        "{guide:?}"
    );
    assert!(
        maa_adapter::navigation_vision::heading(&rgb)
            .unwrap()
            .is_some()
    );
}

#[test]
fn isolated_nearby_icon_does_not_hide_a_connected_route() {
    let mut rgb = RgbImage::new(210, 210);
    for (x, y) in [(119, 105), (68, 105), (56, 105)] {
        for dy in 0..4 {
            for dx in 0..4 {
                rgb.put_pixel(x + dx, y + dy, Rgb([140, 235, 0]));
            }
        }
    }
    let guide = guide_point(&rgb).unwrap();
    assert!(guide.x < 70, "{guide:?}");
}

#[test]
fn isolated_green_icon_and_missing_line_do_not_provide_navigation() {
    let mut rgb = RgbImage::from_pixel(210, 210, Rgb([30, 35, 40]));
    assert!(guide_point(&rgb).is_none());
    for y in 100..106 {
        for x in 130..136 {
            rgb.put_pixel(x, y, Rgb([130, 240, 0]));
        }
    }
    assert!(guide_point(&rgb).is_none());
    assert!(guide_point(&RgbImage::new(267, 274)).is_none());
}

#[test]
fn cornering_uses_near_path_instead_of_distant_destination() {
    let mut rgb = RgbImage::new(210, 210);
    for (x, y) in [(119, 105), (130, 105), (130, 94), (130, 83), (185, 70)] {
        for dy in 0..4 {
            for dx in 0..4 {
                rgb.put_pixel(x + dx, y + dy, Rgb([140, 235, 0]));
            }
        }
    }
    let guide = guide_point(&rgb).unwrap();
    assert!(guide.x < 150 && guide.y >= 80 && guide.y < 106, "{guide:?}");
}
