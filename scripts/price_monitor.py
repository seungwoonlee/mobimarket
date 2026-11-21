#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
MobiMarket 가격 감시 및 3중 알림 스케줄러 (price_monitor.py)
- 목표가(상한선/하한선) 도달 시 디스코드 + Home Assistant 홈미니 + Todoist 3대 채널 동시 통지
- 품목별 30분 쿨다운 및 SQLite WAL 동시성 보호 적용
"""

import os
import sys
import json
import time
import sqlite3
import argparse
import urllib.request
import urllib.parse
from pathlib import Path
from datetime import datetime

# 承雲 시스템 전역 규칙 상수
HA_BASE_URL = os.environ.get("HASS_URL", "http://192.168.0.2:8123")
HA_TOKEN = os.environ.get("HASS_TOKEN", "")
HA_SPEAKER_ENTITY = "media_player.home_mini_1"
HA_SPEAKER_VOLUME = 0.50  # 50% 볼륨

DISCORD_WEBHOOK_URL = os.environ.get("DISCORD_WEBHOOK_URL", "")
TODOIST_API_TOKEN = os.environ.get("TODOIST_API_TOKEN", "")

STATE_FILE = Path("data/alert_state.json")
DEFAULT_RULES_FILE = Path("config/alert_rules.json")

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

def load_alert_rules() -> list:
    if DEFAULT_RULES_FILE.exists():
        try:
            with open(DEFAULT_RULES_FILE, "r", encoding="utf-8") as f:
                return json.load(f)
        except Exception as e:
            print(f"[WARN] 알림 규칙 파일 읽기 실패: {e}")
    
    # 기본 모니터링 규칙
    return [
        {"item_name": "마력 깃든 용비늘", "condition": "<=", "target_price": 320000, "cooldown_mins": 30},
        {"item_name": "허상의 마력석", "condition": "<=", "target_price": 105, "cooldown_mins": 30},
        {"item_name": "포식의 마력석", "condition": "<=", "target_price": 115, "cooldown_mins": 30},
        {"item_name": "심해의 마력석", "condition": "<=", "target_price": 270, "cooldown_mins": 30},
    ]

def load_alert_state() -> dict:
    if STATE_FILE.exists():
        try:
            with open(STATE_FILE, "r", encoding="utf-8") as f:
                return json.load(f)
        except Exception:
            return {}
    return {}

def save_alert_state(state: dict):
    STATE_FILE.parent.mkdir(parents=True, exist_ok=True)
    with open(STATE_FILE, "w", encoding="utf-8") as f:
        json.dump(state, f, ensure_ascii=False, indent=2)

def notify_home_assistant(message: str, dry_run: bool = False):
    """Home Assistant 홈미니 50% 볼륨 TTS 방송 (承雲 시스템 최우선)"""
    if dry_run or not HA_TOKEN:
        print(f"[HA 홈미니 방송 (Dry-Run / 미설정)]: 볼륨 50% -> \"{message}\"")
        return

    try:
        # 1. 볼륨 50% 설정
        vol_req = urllib.request.Request(
            f"{HA_BASE_URL}/api/services/media_player/volume_set",
            data=json.dumps({"entity_id": HA_SPEAKER_ENTITY, "volume_level": HA_SPEAKER_VOLUME}).encode("utf-8"),
            headers={"Authorization": f"Bearer {HA_TOKEN}", "Content-Type": "application/json"}
        )
        urllib.request.urlopen(vol_req, timeout=5)

        # 2. TTS 메시지 방송
        tts_req = urllib.request.Request(
            f"{HA_BASE_URL}/api/services/tts/speak",
            data=json.dumps({
                "media_player_entity_id": HA_SPEAKER_ENTITY,
                "message": message,
                "entity_id": "tts.google_en_com"
            }).encode("utf-8"),
            headers={"Authorization": f"Bearer {HA_TOKEN}", "Content-Type": "application/json"}
        )
        urllib.request.urlopen(tts_req, timeout=5)
        print(f"[HA 홈미니 방송 완료]: {message}")
    except Exception as e:
        print(f"[HA 방송 실패]: {e}")

def notify_discord(message: str, dry_run: bool = False):
    """디스코드 웹훅 알림"""
    if dry_run or not DISCORD_WEBHOOK_URL:
        print(f"[디스코드 알림 (Dry-Run / 미설정)]: \"{message}\"")
        return

    try:
        payload = {"content": f"⚡ **[MobiMarket 시세 알림]**\n{message}"}
        req = urllib.request.Request(
            DISCORD_WEBHOOK_URL,
            data=json.dumps(payload).encode("utf-8"),
            headers={"Content-Type": "application/json"}
        )
        urllib.request.urlopen(req, timeout=5)
        print(f"[디스코드 알림 완료]: {message}")
    except Exception as e:
        print(f"[디스코드 알림 실패]: {e}")

def notify_todoist(content: str, dry_run: bool = False):
    """Todoist 작업 추가"""
    if dry_run or not TODOIST_API_TOKEN:
        print(f"[Todoist 태스크 등록 (Dry-Run / 미설정)]: \"{content}\"")
        return

    try:
        payload = {"content": f"[거래소 알림] {content}", "priority": 4}
        req = urllib.request.Request(
            "https://api.todoist.com/rest/v2/tasks",
            data=json.dumps(payload).encode("utf-8"),
            headers={"Authorization": f"Bearer {TODOIST_API_TOKEN}", "Content-Type": "application/json"}
        )
        urllib.request.urlopen(req, timeout=5)
        print(f"[Todoist 태스크 등록 완료]: {content}")
    except Exception as e:
        print(f"[Todoist 실패]: {e}")

def dispatch_all_notifications(title: str, message: str, dry_run: bool = False):
    """3대 채널 동시 발송 (규칙 준수)"""
    print(f"\n📢 [알림 트리거 발동] {title}")
    notify_home_assistant(f"마비노기 거래소 알림. {message}", dry_run=dry_run)
    notify_discord(f"{title}\n{message}", dry_run=dry_run)
    notify_todoist(f"{title}: {message}", dry_run=dry_run)

def check_prices(dry_run: bool = False):
    db_path = get_db_path()
    if not db_path.exists():
        print(f"[ERROR] DB 파일 없음: {db_path}")
        return

    conn = sqlite3.connect(str(db_path))
    conn.execute("PRAGMA busy_timeout = 5000;")
    cur = conn.cursor()

    rules = load_alert_rules()
    state = load_alert_state()
    now_ts = time.time()

    triggered_count = 0

    for rule in rules:
        item = rule["item_name"]
        cond = rule.get("condition", "<=")
        target = rule["target_price"]
        cooldown_secs = rule.get("cooldown_mins", 30) * 60

        # 최신 시세 조회
        cur.execute("""
            SELECT min_unit_price, in_stock, scraped_at
            FROM market_snapshots
            WHERE item_name = ?
            ORDER BY id DESC LIMIT 1
        """, (item,))
        row = cur.fetchone()
        if not row:
            continue

        price, in_stock, scraped_at = row[0], bool(row[1]), row[2]

        # 조건 충족 여부 검사
        matched = False
        if cond == "<=" and price <= target:
            matched = True
        elif cond == ">=" and price >= target:
            matched = True

        if matched:
            last_alert_ts = state.get(item, {}).get("last_alert_ts", 0)
            if now_ts - last_alert_ts < cooldown_secs:
                remaining = int((cooldown_secs - (now_ts - last_alert_ts)) / 60)
                print(f"[쿨다운 유지] {item}: 현재가 {price:,} 데카 (남은 쿨다운: {remaining}분)")
                continue

            title = f"🎯 {item} 목표가 달성!"
            msg = f"{item} 현재 최저가가 {price:,} 데카(목표: {target:,} 데카 {cond})에 도달했습니다. (수집시각: {scraped_at})"
            dispatch_all_notifications(title, msg, dry_run=dry_run)

            state[item] = {
                "last_alert_ts": now_ts,
                "price": price,
                "time": datetime.now().strftime("%Y-%m-%d %H:%M:%S")
            }
            triggered_count += 1
        else:
            print(f"[정상] {item}: 현재 {price:,} 데카 (목표 {target:,} 데카 {cond})")

    conn.close()
    if not dry_run:
        save_alert_state(state)

    print(f"\n[검사 완료] 총 {len(rules)}개 규칙 검사 완료, {triggered_count}개 알림 발송.")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="MobiMarket 시세 감시 및 3중 알림 스케줄러")
    parser.add_argument("--dry-run", action="store_true", help="실제 외부 API 호출 없이 로컬 모의 알림 실행")
    parser.add_argument("--save-rules", action="store_true", help="기본 알림 규칙 JSON 생성")
    args = parser.parse_args()

    if args.save_rules:
        DEFAULT_RULES_FILE.parent.mkdir(parents=True, exist_ok=True)
        rules = load_alert_rules()
        with open(DEFAULT_RULES_FILE, "w", encoding="utf-8") as f:
            json.dump(rules, f, ensure_ascii=False, indent=2)
        print(f"[OK] {DEFAULT_RULES_FILE}에 규칙이 저장되었습니다.")
        sys.exit(0)

    check_prices(dry_run=args.dry_run)
