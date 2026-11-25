use rusqlite::{params, Connection, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotRecord {
    pub id: i64,
    pub item_name: String,
    pub rarity: Option<String>,
    pub star: Option<i64>,
    pub season: Option<String>,
    pub min_unit_price: i64,
    pub in_stock: bool,
    pub scraped_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeRecord {
    pub display_name: String,
    pub produced_per_craft: i64,
    pub success_rate: f64,
    pub critical_success_rate: f64,
    pub ingredients_json: String,
    pub updated_at: String,
}

#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

fn local_now_str() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn alt_zz_name(item_name: &str) -> String {
    if item_name.ends_with("ZZ") {
        item_name.trim_end_matches("ZZ").to_string()
    } else {
        format!("{}ZZ", item_name)
    }
}

impl Database {
    pub fn new(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)?;

        // 동시성 및 안정성 최적화 PRAGMA + 기본 테이블 생성
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA busy_timeout = 5000;
            PRAGMA synchronous = NORMAL;

            -- 1. 마켓 시세 스냅샷 테이블
            CREATE TABLE IF NOT EXISTS market_snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                item_name TEXT NOT NULL,
                rarity TEXT,
                star INTEGER,
                season TEXT,
                min_unit_price INTEGER NOT NULL,
                in_stock INTEGER NOT NULL,
                scraped_at DATETIME DEFAULT (datetime('now', 'localtime'))
            );
            CREATE INDEX IF NOT EXISTS idx_snapshots_search 
                ON market_snapshots(item_name, rarity, star, season, scraped_at);
            CREATE INDEX IF NOT EXISTS idx_snapshots_item_time 
                ON market_snapshots(item_name, scraped_at);

            -- 2. 제작 레시피 테이블
            CREATE TABLE IF NOT EXISTS recipes (
                display_name TEXT PRIMARY KEY,
                produced_per_craft INTEGER NOT NULL DEFAULT 1,
                success_rate REAL DEFAULT 100.0,
                critical_success_rate REAL DEFAULT 0.0,
                ingredients_json TEXT NOT NULL,
                updated_at DATETIME DEFAULT (datetime('now', 'localtime'))
            );

            -- 3. 사용자 모니터링 품목 테이블
            CREATE TABLE IF NOT EXISTS monitored_items (
                item_name TEXT PRIMARY KEY,
                group_name TEXT,
                added_at DATETIME DEFAULT (datetime('now', 'localtime'))
            );

            -- 4. 사용자 관심 장비/아이템 찜(Favorites) 테이블
            CREATE TABLE IF NOT EXISTS favorites (
                item_name TEXT PRIMARY KEY,
                created_at DATETIME DEFAULT (datetime('now', 'localtime'))
            );
            "#,
        )?;

        // 기본 모니터링 품목 초기 시드 등록 및 폐기된 과거 기본 시드 자동 정리 (KST 기준 타임스탬프)
        let now_local = local_now_str();
        {
            let tx = conn.transaction()?;
            let current_defaults: HashSet<&str> = crate::config::DEFAULT_MONITORED_NAMES.iter().copied().collect();

            // 과거 버전(v1.2.0 등)에서 '기본' 시드로 등록되었다가 config에서 제거된 유령 품목(예: 단단한 무쇠괴) 정리
            let stale_default_items: Vec<String> = {
                let mut stmt = tx.prepare("SELECT item_name FROM monitored_items WHERE group_name = '기본'")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                rows.filter_map(|r| r.ok())
                    .filter(|name| !current_defaults.contains(name.as_str()))
                    .collect()
            };
            for stale in stale_default_items {
                tx.execute(
                    "DELETE FROM monitored_items WHERE item_name = ?1 AND group_name = '기본'",
                    params![stale],
                )?;
            }

            for &name in crate::config::DEFAULT_MONITORED_NAMES {
                let grp = if crate::config::GROUP_MATERIALS.contains(&name) {
                    "재료"
                } else {
                    "기본"
                };
                tx.execute(
                    "INSERT OR IGNORE INTO monitored_items (item_name, group_name, added_at) VALUES (?1, ?2, ?3)",
                    params![name, grp, &now_local],
                )?;
            }
            tx.commit()?;
        }

        // 트랜잭션 기반 멱등 마이그레이션 (실패 시 롤백 및 에러 전파로 이중 보정 원천 차단)
        let mut version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;

        // 마이그레이션 v1: 기존 UTC 저장 스냅샷을 OS 로컬 시간(KST)으로 1회 안전 보정
        if version < 1 {
            let tx = conn.transaction()?;
            tx.execute_batch(
                r#"
                UPDATE market_snapshots 
                SET scraped_at = datetime(scraped_at, 'localtime')
                WHERE scraped_at IS NOT NULL;

                PRAGMA user_version = 1;
                "#,
            )?;
            tx.commit()?;
            version = 1;
        }

        // 마이그레이션 v2: 신규 추가된 레시피 재료 및 가공재료 자동 보충
        if version < 2 {
            let tx = conn.transaction()?;
            for &name in crate::config::GROUP_MATERIALS {
                tx.execute(
                    "INSERT OR IGNORE INTO monitored_items (item_name, group_name, added_at) VALUES (?1, '재료', ?2)",
                    params![name, &now_local],
                )?;
            }
            tx.execute("PRAGMA user_version = 2;", [])?;
            tx.commit()?;
            version = 2;
        }

        // 마이그레이션 v3: 기존 DB 잔영/해연 장비 ZZ 접미사 정규화 및 타 테이블 UTC -> KST 일괄 보정
        if version < 3 {
            let tx = conn.transaction()?;
            tx.execute_batch(
                r#"
                DELETE FROM monitored_items 
                WHERE (item_name LIKE '잔영%' OR item_name LIKE '해연%') 
                  AND item_name NOT LIKE '%ZZ'
                  AND (item_name || 'ZZ') IN (SELECT item_name FROM monitored_items);

                UPDATE monitored_items 
                SET item_name = item_name || 'ZZ' 
                WHERE (item_name LIKE '잔영%' OR item_name LIKE '해연%') 
                  AND item_name NOT LIKE '%ZZ';

                PRAGMA user_version = 3;
                "#,
            )?;
            tx.commit()?;
        }

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// 동기 rusqlite 작업을 Tokio blocking 스레드 풀로 오프로딩하는 공용 헬퍼
    async fn with_conn<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock().unwrap_or_else(|e| e.into_inner());
            f(&mut guard)
        })
        .await
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
    }

    /// 모니터링 품목 목록 조회
    pub async fn get_monitored_items(&self) -> Result<Vec<String>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare("SELECT item_name FROM monitored_items ORDER BY added_at ASC")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut list = Vec::new();
            for r in rows {
                list.push(r?);
            }
            Ok(list)
        })
        .await
    }

    /// 모니터링 품목 추가
    pub async fn add_monitored_item(&self, item_name: &str, group_name: Option<&str>) -> Result<()> {
        let item_name = item_name.to_string();
        let group = group_name.unwrap_or("사용자 추가").to_string();
        let now_local = local_now_str();
        self.with_conn(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO monitored_items (item_name, group_name, added_at) VALUES (?1, ?2, ?3)",
                params![item_name, group, now_local],
            )?;
            Ok(())
        })
        .await
    }

    /// 모니터링 품목이 없을 때만 추가 (기존 등록 일자 및 메타데이터 보존)
    pub async fn add_monitored_item_ignore(&self, item_name: &str, group_name: &str) -> Result<()> {
        let item_name = item_name.to_string();
        let group_name = group_name.to_string();
        let now_local = local_now_str();
        self.with_conn(move |conn| {
            conn.execute(
                "INSERT OR IGNORE INTO monitored_items (item_name, group_name, added_at) VALUES (?1, ?2, ?3)",
                params![item_name, group_name, now_local],
            )?;
            Ok(())
        })
        .await
    }

    /// 모니터링 품목 제거
    pub async fn remove_monitored_item(&self, item_name: &str) -> Result<()> {
        let item_name = item_name.to_string();
        self.with_conn(move |conn| {
            conn.execute("DELETE FROM monitored_items WHERE item_name = ?1", params![item_name])?;
            Ok(())
        })
        .await
    }

    /// 시세 스냅샷 단건 저장 (OS 로컬 타임존 KST 기준)
    pub async fn insert_snapshot(
        &self,
        item_name: &str,
        rarity: Option<&str>,
        star: Option<i64>,
        season: Option<&str>,
        min_unit_price: i64,
        in_stock: bool,
    ) -> Result<()> {
        let item_name = item_name.to_string();
        let rarity = rarity.map(|s| s.to_string());
        let season = season.map(|s| s.to_string());
        let now_local = local_now_str();
        self.with_conn(move |conn| {
            conn.execute(
                r#"
                INSERT INTO market_snapshots (item_name, rarity, star, season, min_unit_price, in_stock, scraped_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                "#,
                params![item_name, rarity, star, season, min_unit_price, in_stock as i64, now_local],
            )?;
            Ok(())
        })
        .await
    }

    /// 특정 아이템의 최신 스냅샷 조회 (ZZ 양방향 단일 쿼리 지원)
    pub async fn get_latest_snapshot(&self, item_name: &str) -> Result<Option<SnapshotRecord>> {
        let name = item_name.to_string();
        let alt = alt_zz_name(&name);
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT id, item_name, rarity, star, season, min_unit_price, in_stock, scraped_at
                FROM market_snapshots
                WHERE item_name IN (?1, ?2)
                ORDER BY scraped_at DESC, id DESC
                LIMIT 1
                "#,
            )?;
            let mut rows = stmt.query(params![name, alt])?;
            if let Some(row) = rows.next()? {
                Ok(Some(SnapshotRecord {
                    id: row.get(0)?,
                    item_name: row.get(1)?,
                    rarity: row.get(2)?,
                    star: row.get(3)?,
                    season: row.get(4)?,
                    min_unit_price: row.get(5)?,
                    in_stock: row.get::<_, i64>(6)? != 0,
                    scraped_at: row.get(7)?,
                }))
            } else {
                Ok(None)
            }
        })
        .await
    }

    /// 모든 아이템의 최신 최저가 일괄 조회
    #[allow(dead_code)]
    pub async fn get_all_latest_prices(&self) -> Result<Vec<(String, i64)>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT item_name, min_unit_price
                FROM market_snapshots
                WHERE id IN (
                    SELECT MAX(id)
                    FROM market_snapshots
                    GROUP BY item_name
                )
                "#,
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            let mut list = Vec::new();
            for r in rows {
                list.push(r?);
            }
            Ok(list)
        })
        .await
    }

    /// 특정 아이템의 최근 N개 스냅샷을 시간순(오름차순)으로 조회 (ZZ 양방향 이력 통합)
    #[allow(dead_code)]
    pub async fn get_snapshot_history(&self, item_name: &str, limit: i64) -> Result<Vec<SnapshotRecord>> {
        let name = item_name.to_string();
        let alt = alt_zz_name(&name);
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT id, item_name, rarity, star, season, min_unit_price, in_stock, scraped_at
                FROM (
                    SELECT id, item_name, rarity, star, season, min_unit_price, in_stock, scraped_at
                    FROM market_snapshots
                    WHERE item_name IN (?1, ?2)
                    ORDER BY scraped_at DESC, id DESC
                    LIMIT ?3
                )
                ORDER BY scraped_at ASC, id ASC
                "#,
            )?;
            let rows = stmt.query_map(params![name, alt, limit], |row| {
                Ok(SnapshotRecord {
                    id: row.get(0)?,
                    item_name: row.get(1)?,
                    rarity: row.get(2)?,
                    star: row.get(3)?,
                    season: row.get(4)?,
                    min_unit_price: row.get(5)?,
                    in_stock: row.get::<_, i64>(6)? != 0,
                    scraped_at: row.get(7)?,
                })
            })?;
            let mut list = Vec::new();
            for r in rows {
                list.push(r?);
            }
            Ok(list)
        })
        .await
    }

    /// 전체 아이템의 최근 N개 스냅샷을 단일 쿼리로 일괄 조회 (N+1 제거, ZZ 접미사 제거 정규화 키로 병합)
    pub async fn get_all_snapshots_grouped(&self, per_item_limit: i64) -> Result<HashMap<String, Vec<SnapshotRecord>>> {
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                r#"
                SELECT id, item_name, rarity, star, season, min_unit_price, in_stock, scraped_at
                FROM (
                    SELECT
                        id, item_name, rarity, star, season, min_unit_price, in_stock, scraped_at,
                        ROW_NUMBER() OVER (PARTITION BY item_name ORDER BY scraped_at DESC, id DESC) AS rn
                    FROM market_snapshots
                )
                WHERE rn <= ?1
                ORDER BY scraped_at ASC, id ASC
                "#,
            )?;
            let rows = stmt.query_map(params![per_item_limit], |row| {
                Ok(SnapshotRecord {
                    id: row.get(0)?,
                    item_name: row.get(1)?,
                    rarity: row.get(2)?,
                    star: row.get(3)?,
                    season: row.get(4)?,
                    min_unit_price: row.get(5)?,
                    in_stock: row.get::<_, i64>(6)? != 0,
                    scraped_at: row.get(7)?,
                })
            })?;

            let mut grouped: HashMap<String, Vec<SnapshotRecord>> = HashMap::new();
            for r in rows {
                let rec = r?;
                let base_key = rec.item_name.trim_end_matches("ZZ").to_string();
                grouped.entry(base_key).or_default().push(rec);
            }

            // ZZ 병합된 목록이 시간순을 유지하고 per_item_limit을 넘지 않도록 정리
            let limit_usize = per_item_limit.max(1) as usize;
            for snaps in grouped.values_mut() {
                snaps.sort_by(|a, b| a.scraped_at.cmp(&b.scraped_at).then_with(|| a.id.cmp(&b.id)));
                if snaps.len() > limit_usize {
                    let drop_n = snaps.len() - limit_usize;
                    snaps.drain(0..drop_n);
                }
            }

            Ok(grouped)
        })
        .await
    }

    /// 레시피 일괄 Upsert
    pub async fn upsert_recipe(
        &self,
        display_name: &str,
        produced_per_craft: i64,
        success_rate: f64,
        critical_success_rate: f64,
        ingredients_json: &str,
    ) -> Result<()> {
        let display_name = display_name.to_string();
        let ingredients_json = ingredients_json.to_string();
        let now_local = local_now_str();
        self.with_conn(move |conn| {
            conn.execute(
                r#"
                INSERT INTO recipes (display_name, produced_per_craft, success_rate, critical_success_rate, ingredients_json, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ON CONFLICT(display_name) DO UPDATE SET
                    produced_per_craft = excluded.produced_per_craft,
                    success_rate = excluded.success_rate,
                    critical_success_rate = excluded.critical_success_rate,
                    ingredients_json = excluded.ingredients_json,
                    updated_at = excluded.updated_at
                "#,
                params![display_name, produced_per_craft, success_rate, critical_success_rate, ingredients_json, now_local],
            )?;
            Ok(())
        })
        .await
    }

    /// 전체 레시피 조회 (잔영 및 해연 필터 지원)
    pub async fn get_recipes(&self, pattern: Option<&str>) -> Result<Vec<RecipeRecord>> {
        let pattern = pattern.map(|p| format!("%{}%", p));
        self.with_conn(move |conn| {
            let mut list = Vec::new();
            if let Some(filter) = pattern {
                let mut stmt = conn.prepare(
                    "SELECT display_name, produced_per_craft, success_rate, critical_success_rate, ingredients_json, updated_at FROM recipes WHERE display_name LIKE ?1 ORDER BY display_name ASC"
                )?;
                let rows = stmt.query_map(params![filter], |row| {
                    Ok(RecipeRecord {
                        display_name: row.get(0)?,
                        produced_per_craft: row.get(1)?,
                        success_rate: row.get(2)?,
                        critical_success_rate: row.get(3)?,
                        ingredients_json: row.get(4)?,
                        updated_at: row.get(5)?,
                    })
                })?;
                for r in rows {
                    list.push(r?);
                }
            } else {
                let mut stmt = conn.prepare(
                    "SELECT display_name, produced_per_craft, success_rate, critical_success_rate, ingredients_json, updated_at FROM recipes ORDER BY display_name ASC"
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok(RecipeRecord {
                        display_name: row.get(0)?,
                        produced_per_craft: row.get(1)?,
                        success_rate: row.get(2)?,
                        critical_success_rate: row.get(3)?,
                        ingredients_json: row.get(4)?,
                        updated_at: row.get(5)?,
                    })
                })?;
                for r in rows {
                    list.push(r?);
                }
            }
            Ok(list)
        })
        .await
    }

    /// 관심 장비/아이템 찜(Favorites) 추가
    pub async fn add_favorite(&self, item_name: &str) -> Result<()> {
        let item_name = item_name.to_string();
        let now_local = local_now_str();
        self.with_conn(move |conn| {
            conn.execute(
                r#"
                INSERT OR IGNORE INTO favorites (item_name, created_at)
                VALUES (?1, ?2)
                "#,
                params![item_name, now_local],
            )?;
            Ok(())
        })
        .await
    }

    /// 관심 장비/아이템 찜(Favorites) 제거
    pub async fn remove_favorite(&self, item_name: &str) -> Result<()> {
        let item_name = item_name.to_string();
        self.with_conn(move |conn| {
            conn.execute(
                "DELETE FROM favorites WHERE item_name = ?1",
                params![item_name],
            )?;
            Ok(())
        })
        .await
    }

    /// 전체 찜(Favorites) 목록 조회
    pub async fn get_all_favorites(&self) -> Result<HashSet<String>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare("SELECT item_name FROM favorites ORDER BY created_at DESC")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut set = HashSet::new();
            for r in rows {
                set.insert(r?);
            }
            Ok(set)
        })
        .await
    }
}
