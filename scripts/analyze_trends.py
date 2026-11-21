#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
MobiMarket 시계열 데이터 분석 및 최적 매수 타이밍 도출 (analyze_trends.py)
- SQLite 스냅샷 데이터 기반 기술적 지표(SMA, EMA, 볼린저 밴드, 변동계수) 계산
- 시간대별/요일별 가격 통계 분석 및 저점 매수 타이밍 도출
"""

import os
import sqlite3
import pandas as pd
import numpy as np
from pathlib import Path
from datetime import datetime

def get_db_path() -> Path:
    env_path = os.environ.get("MOBIMARKET_DB_PATH")
    if env_path and Path(env_path).exists():
        return Path(env_path)
    candidates = [
        Path("target/release/mobi_market.db"),
        Path("target/debug/mobi_market.db"),
        Path("mobi_market.db"),
    ]
    for c in candidates:
        if c.exists():
            return c
    return Path("target/release/mobi_market.db")

def load_data(limit_per_item: int = 500) -> pd.DataFrame:
    db_path = get_db_path()
    conn = sqlite3.connect(str(db_path))
    conn.execute("PRAGMA busy_timeout = 5000;")
    
    query = """
    SELECT item_name, min_unit_price, in_stock, scraped_at
    FROM market_snapshots
    ORDER BY id ASC
    """
    df = pd.read_sql_query(query, conn)
    conn.close()

    df['scraped_at'] = pd.to_datetime(df['scraped_at'])
    df['hour'] = df['scraped_at'].dt.hour
    df['day_name'] = df['scraped_at'].dt.day_name()
    df['date'] = df['scraped_at'].dt.date
    return df

def analyze_item(df: pd.DataFrame, item_name: str):
    sub = df[df['item_name'] == item_name].copy()
    if sub.empty:
        print(f"[!] 품목 '{item_name}'의 데이터가 없습니다.")
        return

    sub.sort_values('scraped_at', inplace=True)
    prices = sub['min_unit_price']

    # 1. 기술적 지표 계산
    sub['SMA_5'] = prices.rolling(window=5, min_periods=1).mean()
    sub['SMA_20'] = prices.rolling(window=20, min_periods=1).mean()
    sub['STD_20'] = prices.rolling(window=20, min_periods=1).std().fillna(0)
    sub['BB_Upper'] = sub['SMA_20'] + (sub['STD_20'] * 2)
    sub['BB_Lower'] = sub['SMA_20'] - (sub['STD_20'] * 2)

    cur_price = prices.iloc[-1]
    mean_price = prices.mean()
    min_price = prices.min()
    max_price = prices.max()
    std_price = prices.std()
    cv = (std_price / mean_price * 100) if mean_price > 0 else 0

    print(f"\n========================================================")
    print(f"📊 [시계열 분석 리포트] {item_name}")
    print(f"========================================================")
    print(f"▶ 샘플 수: {len(sub):,}개 관측치")
    print(f"▶ 현재 최저가: {cur_price:,} 데카")
    print(f"▶ 전체 평균가: {mean_price:,.1f} 데카 (표준편차: ±{std_price:,.1f}, 변동계수: {cv:.2f}%)")
    print(f"▶ 기간 최저/최고: {min_price:,} 데카 ~ {max_price:,} 데카 (스프레드: {(max_price - min_price):,} 데카)")
    print(f"▶ 볼린저 밴드(20, 2σ): 하단 {sub['BB_Lower'].iloc[-1]:,.0f} ~ 상단 {sub['BB_Upper'].iloc[-1]:,.0f}")

    # 2. 매수 포지션 평가
    bb_lower = sub['BB_Lower'].iloc[-1]
    if cur_price <= bb_lower or cur_price <= min_price * 1.02:
        signal = "🔥 강력 매수 추천 (볼린저 하단 접근 또는 기간 바닥권)"
    elif cur_price < mean_price:
        signal = "✅ 분할 매수 적기 (평균가 대비 저렴한 구간)"
    elif cur_price > sub['BB_Upper'].iloc[-1]:
        signal = "⚠️ 과열 구간 (단기 고점, 매수 주의 / 보유분 매도 고려)"
    else:
        signal = "⚖️ 중립 관망 (적정 시세 유지 중)"

    print(f"▶ 포지션 시그널: {signal}")

    # 3. 시간대별 평균 시세 (골든 타임 분석)
    hourly = sub.groupby('hour')['min_unit_price'].agg(['mean', 'count'])
    if len(hourly) > 1:
        best_hour = hourly['mean'].idxmin()
        worst_hour = hourly['mean'].idxmax()
        print(f"▶ 최저가 집중 시간대: {best_hour:02d}시경 (평균 {hourly.loc[best_hour, 'mean']:,.0f} 데카)")
        print(f"▶ 최고가 집중 시간대: {worst_hour:02d}시경 (평균 {hourly.loc[worst_hour, 'mean']:,.0f} 데카)")

def main():
    print("MobiMarket 시계열 데이터 로드 중...")
    df = load_data()
    print(f"총 {len(df):,}개 스냅샷 데이터 로드 완료.")

    key_items = ["마력 깃든 용비늘", "허상의 마력석", "심해의 마력석", "공명의 영혼석"]
    for item in key_items:
        analyze_item(df, item)

if __name__ == "__main__":
    main()
