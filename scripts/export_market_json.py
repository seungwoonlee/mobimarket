#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
MobiMarket 데이터 추출 유틸리티 (export_market_json.py)
SQLite 데이터베이스에서 최신 시세 및 레시피 정보를 JSON 파일로 내보냅니다.
"""

import os
import json
import sqlite3
from pathlib import Path

def get_db_path() -> Path:
    env_path = os.environ.get("MOBIMARKET_DB_PATH")
    if env_path and Path(env_path).exists():
        return Path(env_path)
    
    # 후보 경로 탐색
    candidates = [
        Path("target/release/mobi_market.db"),
        Path("target/debug/mobi_market.db"),
        Path("mobi_market.db"),
    ]
    for c in candidates:
        if c.exists():
            return c
    return Path("target/release/mobi_market.db")

def export_data(output_path: str = "web/market_data.json"):
    db_path = get_db_path()
    if not db_path.exists():
        print(f"[ERROR] DB 파일을 찾을 수 없습니다: {db_path}")
        return

    conn = sqlite3.connect(str(db_path))
    conn.execute("PRAGMA busy_timeout = 5000;")
    cur = conn.cursor()

    # 1. 모니터링 품목 및 최신 시세
    cur.execute("""
        SELECT item_name, min_unit_price, in_stock, scraped_at
        FROM market_snapshots
        WHERE id IN (
            SELECT MAX(id) FROM market_snapshots GROUP BY item_name
        )
        ORDER BY item_name ASC
    """)
    latest_items = []
    for row in cur.fetchall():
        latest_items.append({
            "name": row[0],
            "price": row[1],
            "in_stock": bool(row[2]),
            "scraped_at": row[3]
        })

    # 2. 최근 7일 스냅샷 히스토리 (주요 품목 위주)
    cur.execute("""
        SELECT item_name, min_unit_price, scraped_at
        FROM market_snapshots
        ORDER BY id ASC
    """)
    history = {}
    for row in cur.fetchall():
        name, price, dt = row[0], row[1], row[2]
        if name not in history:
            history[name] = []
        history[name].append({"price": price, "time": dt})

    # 3. 제작 레시피
    cur.execute("""
        SELECT display_name, produced_per_craft, success_rate, critical_success_rate, ingredients_json
        FROM recipes
        ORDER BY display_name ASC
    """)
    recipes = []
    for row in cur.fetchall():
        try:
            ingredients = json.loads(row[4])
        except Exception:
            ingredients = []
        recipes.append({
            "name": row[0],
            "produced": row[1],
            "success_rate": row[2],
            "critical_rate": row[3],
            "ingredients": ingredients
        })

    conn.close()

    data = {
        "latest_items": latest_items,
        "history": history,
        "recipes": recipes,
    }

    out_file = Path(output_path)
    out_file.parent.mkdir(parents=True, exist_ok=True)
    with open(out_file, "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False, indent=2)

    print(f"[OK] {len(latest_items)}개 품목, {len(recipes)}개 레시피가 {out_file}에 저장되었습니다.")

if __name__ == "__main__":
    export_data()
