"""空洞探索的绿点识别、导航选择和到点策略，不直接操作键鼠。"""
from __future__ import annotations

import base64
import math
from collections import deque
from dataclasses import dataclass, fields
from typing import Any


@dataclass(frozen=True)
class Settings:
    green_min: int = 135
    green_red_ratio: float = 1.12
    green_blue_ratio: float = 1.5
    branch_angle: float = 45.0
    seed_max_distance: float = 38.0
    dot_max_gap: float = 20.0
    road_progress_pixels: float = 2.0
    marker_progress_meters: float = 0.5
    no_progress_ms: int = 4000
    marker_max_meters: float = 35.0

    @classmethod
    def load(cls, values: dict[str, Any]) -> Settings:
        allowed = {field.name for field in fields(cls)}
        if values.keys() - allowed:
            raise ValueError(f"未知设置：{sorted(values.keys() - allowed)}")
        result = cls(**values)
        for field in fields(cls):
            value = getattr(result, field.name)
            if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value <= 0:
                raise ValueError(f"设置必须是有限正数：{field.name}")
        if not 1 <= result.green_min <= 255 or not 10 <= result.branch_angle <= 90:
            raise ValueError("绿色亮度或分支夹角超出范围")
        if not 8 <= result.seed_max_distance <= 50 or not 5 <= result.dot_max_gap <= 25:
            raise ValueError("绿点距离超出局部导航范围")
        if not 250 <= result.no_progress_ms <= 4000 or not 3 < result.marker_max_meters <= 35:
            raise ValueError("开发参数不能放宽底层停滞时间或世界标记距离上限")
        return result


def distance(point: tuple[int, int], other: tuple[int, int] = (105, 105)) -> float:
    return math.hypot(point[0] - other[0], point[1] - other[1])


def guide_routes(rgb: bytes, settings: Settings) -> list[tuple[int, int]]:
    """在 210×210 RGB 小地图中寻找玩家周围独立方向的点串。"""
    if len(rgb) != 210 * 210 * 3:
        raise ValueError("小地图须为 210×210 RGB8")
    mask = bytearray(210 * 210)
    for at in range(210 * 210):
        x, y = at % 210, at // 210
        r, g, b = rgb[at * 3:at * 3 + 3]
        if (x - 105) ** 2 + (y - 105) ** 2 < 94 ** 2 and g >= settings.green_min and g > r * settings.green_red_ratio and g > b * settings.green_blue_ratio:
            mask[at] = 1
    dots: list[tuple[int, int]] = []
    for at in range(len(mask)):
        if not mask[at]:
            continue
        queue = deque([at])
        mask[at] = 0
        count = sx = sy = 0
        minx = miny = 210
        maxx = maxy = 0
        while queue:
            current = queue.popleft()
            x, y = current % 210, current // 210
            count += 1
            sx += x
            sy += y
            minx, maxx = min(minx, x), max(maxx, x)
            miny, maxy = min(miny, y), max(maxy, y)
            for yy in range(max(0, y - 1), min(210, y + 2)):
                for xx in range(max(0, x - 1), min(210, x + 2)):
                    index = yy * 210 + xx
                    if mask[index]:
                        mask[index] = 0
                        queue.append(index)
        width, height = maxx - minx + 1, maxy - miny + 1
        if 4 <= count <= 100 and 2 <= width <= 13 and 2 <= height <= 13 and max(width, height) <= min(width, height) * 2:
            dots.append((sx // count, sy // count))
    dots.sort(key=distance)
    routes: list[tuple[int, int]] = []
    for seed in dots:
        if not 8 <= distance(seed) <= settings.seed_max_distance:
            continue
        current, connected = seed, 1
        while connected < 2 or distance(current) < 30:
            candidates = [p for p in dots if distance(p) > distance(current) + 0.5 and distance(p, current) <= settings.dot_max_gap]
            if not candidates:
                break
            current = min(candidates, key=lambda p: distance(p, current))
            connected += 1
        if connected >= 2:
            bearing = math.degrees(math.atan2(current[1] - 105, current[0] - 105))
            if not any(abs((math.degrees(math.atan2(p[1] - 105, p[0] - 105)) - bearing + 180) % 360 - 180) < settings.branch_angle for p in routes):
                routes.append(current)
    return routes


class ExplorationPolicy:
    def __init__(self, settings: Settings) -> None:
        self.settings = settings
        self.epoch: str | None = None
        self.guide_untrusted: bool = False
        self.progress: tuple[str, float, int] | None = None
        self.blocked: bool = False

    def _progress_stopped(self, mode: str, remaining: float, now: int, threshold: float) -> bool:
        if self.progress is None or self.progress[0] != mode:
            self.progress = (mode, remaining, now)
            self.blocked = False
        _, best, since = self.progress
        if remaining < best - threshold:
            self.progress = (mode, remaining, now)
        elif now - since >= self.settings.no_progress_ms:
            self.blocked = True
        return self.blocked

    def decide(self, request: dict[str, Any]) -> dict[str, Any]:
        context, sight = request["context"], request["sight"]
        epoch = str(context["epoch"])
        if epoch != self.epoch:
            self.epoch, self.guide_untrusted, self.progress, self.blocked = epoch, False, None, False
        if not sight.get("fresh") or not sight.get("world_controls") or sight.get("map_open"):
            return {"action": "wait", "reason": "等待有效探索画面"}
        if sight.get("interact"):
            return {"action": "interact", "reason": "目标名称和交互提示已核对"}
        if sight.get("arrival_point"):
            action = "face" if context["stage"] == "ApproachTarget" else "arrive"
            return {"action": action, "reason": "到点后对准镜头和人物朝向"}
        if context["stage"] == "ApproachTarget":
            return {"action": "recover", "reason": "到点标记消失，重新读取追踪"}
        routes: list[tuple[int, int]] = []
        if encoded := sight.get("mini_rgb_b64"):
            routes = guide_routes(base64.b64decode(encoded, validate=True), self.settings)
        elif point := sight.get("guide"):
            routes = [(point["x"], point["y"])]
        self.guide_untrusted |= len(routes) > 1 or bool(sight.get("guide_ambiguous"))
        if not self.guide_untrusted and len(routes) == 1 and sight.get("heading") is not None:
            self.progress, self.blocked = None, False
            return {"action": "guide", "guide": {"x": routes[0][0], "y": routes[0][1]}, "reason": "跟随唯一绿点路线"}
        now = int(context["time_ms"])
        if (road := sight.get("road_route")) and sight.get("heading") is not None:
            if not self._progress_stopped("road", float(road["remaining"]), now, self.settings.road_progress_pixels):
                return {"action": "road", "reason": "沿底层已验证的连通道路移动"}
        elif (marker := sight.get("world_marker")) and context["target"] == "observation" and 3 <= marker["meters"] <= self.settings.marker_max_meters:
            if not self._progress_stopped("marker", float(marker["meters"]), now, self.settings.marker_progress_meters):
                return {"action": "marker", "reason": "忽略异常绿线，接近已核对的世界标记"}
        return {"action": "recover", "reason": "导航不可用或距离停滞，先松键并恢复追踪"}
