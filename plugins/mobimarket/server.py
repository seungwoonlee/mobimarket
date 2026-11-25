#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
MobiMarket MCP 서버 (server.py)
Antigravity 에이전트가 마비노기 모바일 거래소 시세 및 제작 마진을 실시간으로 조회할 수 있는 MCP 서버입니다.
표준 JSON-RPC 2.0 stdio 프로토콜을 완벽히 지원합니다.
"""

import os
import sys
import json
import sqlite3
from contextlib import closing
from pathlib import Path

def get_db_path() -> Path | None:
    env_path = os.environ.get("MOBIMARKET_DB_PATH")
    if env_path:
        p = Path(env_path)
        return p if p.exists() else None

    root_dir = Path(__file__).resolve().parent.parent.parent
    candidates = [
        Path("data/mobi_market.db"),
        Path("target/release/mobi_market.db"),
        Path("target/debug/mobi_market.db"),
        Path("mobi_market.db"),
        root_dir / "data" / "mobi_market.db",
        root_dir / "target" / "release" / "mobi_market.db",
        root_dir / "target" / "debug" / "mobi_market.db",
        root_dir / "mobi_market.db",
    ]
    for c in candidates:
        if c.exists():
            return c
    return None

def get_connection():
    db_path = get_db_path()
    if db_path is None:
        raise FileNotFoundError("MobiMarket SQLite DB 파일을 찾을 수 없습니다. (MOBIMARKET_DB_PATH 확인 필요)")
    conn = sqlite3.connect(f"file:{db_path.resolve()}?mode=ro", uri=True)
    conn.execute("PRAGMA busy_timeout = 5000;")
    return conn

def get_zz_variants(name: str) -> list[str]:
    if name.endswith("ZZ"):
        return [name, name[:-2]]
    return [name, name + "ZZ"]

def is_zero_cost_material(name: str) -> bool:
    """기본적으로 모든 유저가 넘치게 보유하여 구매 가치가 없는 재료 판별 (원가 0데카 취급)"""
    trimmed = name.strip()
    while trimmed.endswith("ZZ"):
        trimmed = trimmed[:-2]
    return trimmed == "세공된 페리도트"

def fetch_latest_snapshot(cur, item_name: str):
    variants = get_zz_variants(item_name)
    placeholders = ",".join("?" for _ in variants)
    cur.execute(
        f"""
        SELECT item_name, min_unit_price, in_stock, scraped_at
        FROM market_snapshots
        WHERE item_name IN ({placeholders})
        ORDER BY scraped_at DESC, id DESC
        LIMIT 1
        """,
        variants,
    )
    return cur.fetchone()

# 1. 도구: 특정 품목 시세 조회
def tool_get_price(item_name: str) -> dict:
    with closing(get_connection()) as conn:
        cur = conn.cursor()
        latest = fetch_latest_snapshot(cur, item_name)
        if not latest:
            return {"error": f"품목 '{item_name}'의 시세 기록을 찾을 수 없습니다."}

        matched_name, price, in_stock, scraped_at = latest[0], latest[1], bool(latest[2]), latest[3]
        variants = get_zz_variants(matched_name)
        placeholders = ",".join("?" for _ in variants)
        cur.execute(
            f"""
            SELECT MIN(min_unit_price), MAX(min_unit_price), AVG(min_unit_price), COUNT(*)
            FROM market_snapshots
            WHERE item_name IN ({placeholders})
            """,
            variants,
        )
        stats = cur.fetchone()

    return {
        "item_name": matched_name,
        "current_price": price,
        "in_stock": in_stock,
        "scraped_at": scraped_at,
        "period_min_price": stats[0],
        "period_max_price": stats[1],
        "period_avg_price": round(stats[2], 1) if stats[2] is not None else price,
        "snapshot_count": stats[3],
    }

# 2. 도구: 모니터링 품목 목록 및 시세 요약
def tool_list_monitored() -> dict:
    with closing(get_connection()) as conn:
        cur = conn.cursor()
        cur.execute("""
            SELECT s.item_name, s.min_unit_price, s.in_stock, s.scraped_at
            FROM market_snapshots s
            INNER JOIN (
                SELECT item_name, MAX(id) AS max_id
                FROM market_snapshots
                GROUP BY item_name
            ) latest ON s.id = latest.max_id
            ORDER BY s.item_name ASC
        """)
        items = [
            {
                "name": r[0],
                "current_price": r[1],
                "in_stock": bool(r[2]),
                "scraped_at": r[3],
            }
            for r in cur.fetchall()
        ]
    return {"total_count": len(items), "items": items}

# 3. 도구: 제작 레시피 마진 및 순익 계산
def tool_calculate_craft(recipe_name: str, success_override: float = None) -> dict:
    with closing(get_connection()) as conn:
        cur = conn.cursor()
        variants = get_zz_variants(recipe_name)
        placeholders = ",".join("?" for _ in variants)
        cur.execute(
            f"""
            SELECT display_name, produced_per_craft, success_rate, critical_success_rate, ingredients_json
            FROM recipes
            WHERE display_name IN ({placeholders})
            LIMIT 1
            """,
            variants,
        )
        row = cur.fetchone()
        if not row:
            return {"error": f"제작 레시피 '{recipe_name}'을(를) 찾을 수 없습니다."}

        matched_name, produced, succ_rate, crit_rate, ing_json = (
            row[0],
            max(1, int(row[1] or 1)),
            float(row[2] or 0.0),
            float(row[3] or 0.0),
            row[4],
        )
        if success_override is not None:
            succ_rate = float(success_override)

        # 0~1 vs 0~100 정규화
        norm_succ = succ_rate * 100.0 if 0.0 < succ_rate <= 1.0 else succ_rate
        norm_crit = crit_rate * 100.0 if 0.0 < crit_rate <= 1.0 else crit_rate

        try:
            ingredients = json.loads(ing_json)
        except Exception:
            ingredients = []

        total_mat_cost = 0
        missing_prices = []
        ing_details = []
        for ing in ingredients:
            ing_name = ing.get("DisplayName", "")
            req_count = int(ing.get("Required", 1))
            if is_zero_cost_material(ing_name):
                unit_price = 0
                cost = 0
            else:
                snap = fetch_latest_snapshot(cur, ing_name)
                unit_price = snap[1] if snap else None

                if unit_price is None:
                    missing_prices.append(ing_name)
                    cost = None
                else:
                    cost = req_count * unit_price
                    total_mat_cost += cost

            ing_details.append({
                "name": ing_name,
                "required": req_count,
                "unit_price": unit_price,
                "cost": cost,
            })

        prod_snap = fetch_latest_snapshot(cur, matched_name)
        market_price = prod_snap[1] if prod_snap else None

    fee_rate = 0.10
    p_succ = max(0.0, min(1.0, norm_succ / 100.0))
    p_crit = max(0.0, min(1.0, norm_crit / 100.0))
    expected_yield = p_succ * (produced + p_crit)

    if market_price is None or missing_prices:
        if market_price is None:
            missing_prices.append(matched_name)
        return {
            "recipe_name": matched_name,
            "produced_per_craft": produced,
            "success_rate_percent": round(norm_succ, 2),
            "critical_rate_percent": round(norm_crit, 2),
            "expected_yield_per_craft": round(expected_yield, 2),
            "total_material_cost": None if missing_prices and len(missing_prices) == len(ing_details) else total_mat_cost,
            "finished_item_market_price": market_price,
            "expected_net_profit": None,
            "roi_percent": None,
            "is_profitable": None,
            "missing_prices": sorted(set(missing_prices)),
            "warning": "시세 미수집 품목이 있어 정확한 수익 계산이 불가능합니다.",
            "ingredients": ing_details,
        }

    expected_revenue = expected_yield * market_price * (1.0 - fee_rate)
    net_profit = expected_revenue - total_mat_cost
    roi = (net_profit / total_mat_cost * 100.0) if total_mat_cost > 0 else 0.0

    sell_materials_net = round(total_mat_cost * (1.0 - fee_rate))
    product_buy_cost = round(market_price * produced)
    # 전량 보유 가정 시 직접 제작 추가 지출은 0이므로, (완제품가 - 재료 매각 실수령)이 직접 제작 유리액
    craft_advantage_if_owned = product_buy_cost - sell_materials_net
    if product_buy_cost > total_mat_cost:
        recommended_action = "craft"
    elif product_buy_cost < total_mat_cost:
        recommended_action = "buy"
    else:
        recommended_action = "equal"

    if craft_advantage_if_owned > 0:
        recommended_action_if_owned = "craft"
    elif craft_advantage_if_owned < 0:
        recommended_action_if_owned = "sell_materials_and_buy"
    else:
        recommended_action_if_owned = "equal"

    return {
        "recipe_name": matched_name,
        "produced_per_craft": produced,
        "success_rate_percent": round(norm_succ, 2),
        "critical_rate_percent": round(norm_crit, 2),
        "expected_yield_per_craft": round(expected_yield, 2),
        "total_material_cost": total_mat_cost,
        "finished_item_market_price": market_price,
        "expected_net_profit": round(net_profit),
        "roi_percent": round(roi, 1),
        "is_profitable": net_profit > 0,
        "missing_prices": [],
        "craft_vs_buy": {
            "buy_product_price": product_buy_cost,
            "craft_total_material_cost": total_mat_cost,
            "sell_materials_net_if_owned": sell_materials_net,
            "craft_advantage_over_sell_buy_if_owned": craft_advantage_if_owned,
            "recommended_action": recommended_action,
            "recommended_action_if_owned": recommended_action_if_owned,
            "net_saving": product_buy_cost - total_mat_cost,
        },
        "ingredients": ing_details,
    }

TOOLS_SPEC = [
    {
        "name": "mobi_market_get_price",
        "description": "마비노기 모바일 특정 아이템의 현재 최저가, 기간 최저/최고/평균 시세 및 재고 상태를 조회합니다.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "item_name": {"type": "string", "description": "조회할 아이템 이름 (예: '마력 깃든 용비늘', '잔영의 샤프 듀얼소드ZZ')"}
            },
            "required": ["item_name"]
        }
    },
    {
        "name": "mobi_market_list_monitored",
        "description": "거래소에서 현재 모니터링 중인 전체 주요 품목의 최신 시세 리스트를 반환합니다.",
        "inputSchema": {
            "type": "object",
            "properties": {}
        }
    },
    {
        "name": "mobi_market_calculate_craft",
        "description": "제작 레시피의 실시간 재료 시세, 성공률, 대성공 보너스, 경매장 수수료(10%)를 반영한 기대 순이익과 ROI를 계산합니다.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "recipe_name": {"type": "string", "description": "제작할 완제품 이름 (예: '잔영의 샤프 듀얼소드ZZ')"},
                "success_override": {"type": "number", "description": "성공 확률 직접 지정 (옵션, 0~100)"}
            },
            "required": ["recipe_name"]
        }
    }
]

def main():
    while True:
        line = sys.stdin.readline()
        if not line:
            break
        line = line.strip()
        if not line:
            continue

        try:
            req = json.loads(line)
        except Exception:
            continue

        if not isinstance(req, dict):
            continue

        msg_id = req.get("id")
        method = req.get("method")
        params = req.get("params") or {}

        # JSON-RPC Notification (id가 없는 요청, 예: notifications/initialized)에는 응답을 송출하지 않음
        if msg_id is None:
            continue

        if method == "initialize":
            res = {
                "jsonrpc": "2.0",
                "id": msg_id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {}},
                    "serverInfo": {
                        "name": "mobimarket-mcp",
                        "version": "1.0.0"
                    }
                }
            }
        elif method == "tools/list":
            res = {
                "jsonrpc": "2.0",
                "id": msg_id,
                "result": {
                    "tools": TOOLS_SPEC
                }
            }
        elif method == "tools/call":
            tool_name = params.get("name")
            args = params.get("arguments") or {}

            try:
                if tool_name == "mobi_market_get_price":
                    content = tool_get_price(args.get("item_name", ""))
                elif tool_name == "mobi_market_list_monitored":
                    content = tool_list_monitored()
                elif tool_name == "mobi_market_calculate_craft":
                    content = tool_calculate_craft(args.get("recipe_name", ""), args.get("success_override"))
                else:
                    content = {"error": f"알 수 없는 도구: {tool_name}"}

                res = {
                    "jsonrpc": "2.0",
                    "id": msg_id,
                    "result": {
                        "content": [
                            {"type": "text", "text": json.dumps(content, ensure_ascii=False, indent=2)}
                        ]
                    }
                }
            except Exception as e:
                res = {
                    "jsonrpc": "2.0",
                    "id": msg_id,
                    "error": {"code": -32603, "message": str(e)}
                }
        else:
            res = {
                "jsonrpc": "2.0",
                "id": msg_id,
                "error": {"code": -32601, "message": f"지원하지 않는 메서드: {method}"}
            }

        sys.stdout.write(json.dumps(res, ensure_ascii=False) + "\n")
        sys.stdout.flush()

if __name__ == "__main__":
    main()
