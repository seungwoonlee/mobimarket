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
from pathlib import Path

def get_db_path() -> Path:
    env_path = os.environ.get("MOBIMARKET_DB_PATH")
    if env_path and Path(env_path).exists():
        return Path(env_path)
    
    # 후보 탐색 (루트 기준 및 상대 경로)
    candidates = [
        Path("target/release/mobi_market.db"),
        Path("target/debug/mobi_market.db"),
        Path("mobi_market.db"),
        Path(__file__).resolve().parent.parent.parent / "target" / "release" / "mobi_market.db",
        Path(__file__).resolve().parent.parent.parent / "mobi_market.db"
    ]
    for c in candidates:
        if c.exists():
            return c
    return Path("target/release/mobi_market.db")

def get_connection():
    db_path = get_db_path()
    conn = sqlite3.connect(str(db_path))
    conn.execute("PRAGMA busy_timeout = 5000;")
    return conn

# 1. 도구: 특정 품목 시세 조회
def tool_get_price(item_name: str) -> dict:
    conn = get_connection()
    cur = conn.cursor()

    # ZZ 양방향 Fallback
    target_names = [item_name]
    if item_name.endswith("ZZ"):
        target_names.append(item_name[:-2])
    else:
        target_names.append(item_name + "ZZ")

    placeholders = ",".join("?" for _ in target_names)
    cur.execute(f"""
        SELECT item_name, min_unit_price, in_stock, scraped_at
        FROM market_snapshots
        WHERE item_name IN ({placeholders})
        ORDER BY id DESC LIMIT 1
    """, target_names)
    latest = cur.fetchone()

    if not latest:
        conn.close()
        return {"error": f"품목 '{item_name}'의 시세 기록을 찾을 수 없습니다."}

    matched_name, price, in_stock, scraped_at = latest[0], latest[1], bool(latest[2]), latest[3]

    # 기간 통계
    cur.execute(f"""
        SELECT MIN(min_unit_price), MAX(min_unit_price), AVG(min_unit_price), COUNT(*)
        FROM market_snapshots
        WHERE item_name = ?
    """, (matched_name,))
    stats = cur.fetchone()
    conn.close()

    return {
        "item_name": matched_name,
        "current_price": price,
        "in_stock": in_stock,
        "scraped_at": scraped_at,
        "period_min_price": stats[0],
        "period_max_price": stats[1],
        "period_avg_price": round(stats[2], 1) if stats[2] else price,
        "snapshot_count": stats[3]
    }

# 2. 도구: 모니터링 품목 목록 및 시세 요약
def tool_list_monitored() -> dict:
    conn = get_connection()
    cur = conn.cursor()

    cur.execute("""
        SELECT item_name, min_unit_price, in_stock, scraped_at
        FROM market_snapshots
        WHERE id IN (
            SELECT MAX(id) FROM market_snapshots GROUP BY item_name
        )
        ORDER BY item_name ASC
    """)
    items = []
    for r in cur.fetchall():
        items.append({
            "name": r[0],
            "current_price": r[1],
            "in_stock": bool(r[2]),
            "scraped_at": r[3]
        })
    conn.close()
    return {"total_count": len(items), "items": items}

# 3. 도구: 제작 레시피 마진 및 순익 계산
def tool_calculate_craft(recipe_name: str, success_override: float = None) -> dict:
    conn = get_connection()
    cur = conn.cursor()

    # ZZ 양방향 Fallback
    target_names = [recipe_name]
    if recipe_name.endswith("ZZ"):
        target_names.append(recipe_name[:-2])
    else:
        target_names.append(recipe_name + "ZZ")

    placeholders = ",".join("?" for _ in target_names)
    cur.execute(f"""
        SELECT display_name, produced_per_craft, success_rate, critical_success_rate, ingredients_json
        FROM recipes
        WHERE display_name IN ({placeholders})
        LIMIT 1
    """, target_names)
    row = cur.fetchone()

    if not row:
        conn.close()
        return {"error": f"제작 레시피 '{recipe_name}'을(를) 찾을 수 없습니다."}

    matched_name, produced, succ_rate, crit_rate, ing_json = row[0], row[1], row[2], row[3], row[4]
    if success_override is not None:
        succ_rate = success_override

    try:
        ingredients = json.loads(ing_json)
    except Exception:
        ingredients = []

    # 재료비 산출
    total_mat_cost = 0
    ing_details = []
    for ing in ingredients:
        ing_name = ing.get("DisplayName", "")
        req_count = ing.get("Required", 1)

        # 재료 최신 시세 조회
        cur.execute("""
            SELECT min_unit_price FROM market_snapshots
            WHERE item_name = ? ORDER BY id DESC LIMIT 1
        """, (ing_name,))
        p_row = cur.fetchone()
        unit_price = p_row[0] if p_row else 100  # Fallback

        cost = req_count * unit_price
        total_mat_cost += cost
        ing_details.append({
            "name": ing_name,
            "required": req_count,
            "unit_price": unit_price,
            "cost": cost
        })

    # 완제품 최신 판매 시세 조회
    cur.execute("""
        SELECT min_unit_price FROM market_snapshots
        WHERE item_name = ? ORDER BY id DESC LIMIT 1
    """, (matched_name,))
    prod_row = cur.fetchone()
    market_price = prod_row[0] if prod_row else (total_mat_cost * 1.5)

    conn.close()

    # 기대 수익 계산 (거래소 수수료 10% 반영)
    fee_rate = 0.10
    p_succ = succ_rate / 100.0
    p_crit = (succ_rate / 100.0) * (crit_rate / 100.0)
    expected_yield = p_succ + p_crit

    expected_revenue = expected_yield * market_price * (1.0 - fee_rate)
    net_profit = expected_revenue - total_mat_cost
    roi = (net_profit / total_mat_cost * 100.0) if total_mat_cost > 0 else 0.0

    return {
        "recipe_name": matched_name,
        "success_rate_percent": succ_rate,
        "critical_rate_percent": crit_rate,
        "expected_yield_per_craft": round(expected_yield, 2),
        "total_material_cost": total_mat_cost,
        "finished_item_market_price": market_price,
        "expected_net_profit": round(net_profit),
        "roi_percent": round(roi, 1),
        "is_profitable": net_profit > 0,
        "ingredients": ing_details
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

        msg_id = req.get("id")
        method = req.get("method")
        params = req.get("params", {})

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
            args = params.get("arguments", {})

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
