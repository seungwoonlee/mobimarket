#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
MobiMarket 데이터 추출 유틸리티 (export_market_json.py)
SQLite 데이터베이스에서 최신 시세 및 레시피 정보를 JSON 파일로 내보냅니다.
"""

import os
import json
import sqlite3
from contextlib import closing
from pathlib import Path

def get_db_path() -> Path | None:
    env_path = os.environ.get("MOBIMARKET_DB_PATH")
    if env_path:
        p = Path(env_path)
        return p if p.exists() else None

    root_dir = Path(__file__).resolve().parent.parent
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

def export_data(output_path: str = "web/market_data.json"):
    db_path = get_db_path()
    if db_path is None:
        print("[ERROR] MobiMarket SQLite DB 파일을 찾을 수 없습니다.")
        return

    with closing(sqlite3.connect(f"file:{db_path.resolve()}?mode=ro", uri=True)) as conn:
        conn.execute("PRAGMA busy_timeout = 5000;")
        cur = conn.cursor()

        # 1. 모니터링 품목 및 최신 시세
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
        latest_items = [
            {
                "name": row[0],
                "price": row[1],
                "in_stock": bool(row[2]),
                "scraped_at": row[3],
            }
            for row in cur.fetchall()
        ]

        # 2. 스냅샷 히스토리 (시간순 정렬)
        cur.execute("""
            SELECT item_name, min_unit_price, scraped_at
            FROM market_snapshots
            ORDER BY scraped_at ASC, id ASC
        """)
        history = {}
        for row in cur.fetchall():
            name, price, dt = row[0], row[1], row[2]
            history.setdefault(name, []).append({"price": price, "time": dt})

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
                "ingredients": ingredients,
            })

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
