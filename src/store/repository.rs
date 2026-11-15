use rusqlite::{params, Connection, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

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

impl Database {
    pub fn new(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;

        // 동시성 및 안정성 최적화 PRAGMA
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
                scraped_at DATETIME DEFAULT CURRENT_TIMESTAMP
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
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

            -- 3. 사용자 모니터링 품목 테이블
            CREATE TABLE IF NOT EXISTS monitored_items (
                item_name TEXT PRIMARY KEY,
                group_name TEXT,
                added_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            "#,
        )?;

        // 기본 모니터링 품목 초기 시드 등록
        for &name in crate::config::DEFAULT_MONITORED_NAMES {
            conn.execute(
                "INSERT OR IGNORE INTO monitored_items (item_name, group_name) VALUES (?1, '기본')",
                params![name],
            )?;
        }

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// 모니터링 품목 목록 조회
    pub async fn get_monitored_items(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare("SELECT item_name FROM monitored_items ORDER BY added_at ASC")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    /// 모니터링 품목 추가
    pub async fn add_monitored_item(&self, item_name: &str, group_name: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT OR REPLACE INTO monitored_items (item_name, group_name, added_at) VALUES (?1, ?2, CURRENT_TIMESTAMP)",
            params![item_name, group_name.unwrap_or("사용자 추가")],
        )?;
        Ok(())
    }

    /// 모니터링 품목 제거
    pub async fn remove_monitored_item(&self, item_name: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute("DELETE FROM monitored_items WHERE item_name = ?1", params![item_name])?;
        Ok(())
    }

    /// 시세 스냅샷 단건 저장
    pub async fn insert_snapshot(
        &self,
        item_name: &str,
        rarity: Option<&str>,
        star: Option<i64>,
        season: Option<&str>,
        min_unit_price: i64,
        in_stock: bool,
    ) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            r#"
            INSERT INTO market_snapshots (item_name, rarity, star, season, min_unit_price, in_stock, scraped_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, CURRENT_TIMESTAMP)
            "#,
            params![item_name, rarity, star, season, min_unit_price, in_stock as i64],
        )?;
        Ok(())
    }

    /// 특정 아이템의 최신 스냅샷 조회
    pub async fn get_latest_snapshot(&self, item_name: &str) -> Result<Option<SnapshotRecord>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, item_name, rarity, star, season, min_unit_price, in_stock, scraped_at
            FROM market_snapshots
            WHERE item_name = ?1
            ORDER BY scraped_at DESC, id DESC
            LIMIT 1
            "#,
        )?;

        let mut rows = stmt.query(params![item_name])?;
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
    }

    /// 모든 아이템의 최신 최저가 일괄 조회
    #[allow(dead_code)]
    pub async fn get_all_latest_prices(&self) -> Result<Vec<(String, i64)>> {
        let conn = self.conn.lock().await;
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
    }

    /// 특정 아이템의 최근 N개 스냅샷을 시간순(오름차순)으로 조회 (서브쿼리 적용으로 최신 데이터 보장)
    pub async fn get_snapshot_history(&self, item_name: &str, limit: i64) -> Result<Vec<SnapshotRecord>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, item_name, rarity, star, season, min_unit_price, in_stock, scraped_at
            FROM (
                SELECT id, item_name, rarity, star, season, min_unit_price, in_stock, scraped_at
                FROM market_snapshots
                WHERE item_name = ?1
                ORDER BY scraped_at DESC, id DESC
                LIMIT ?2
            )
            ORDER BY scraped_at ASC, id ASC
            "#,
        )?;

        let rows = stmt.query_map(params![item_name, limit], |row| {
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
        let conn = self.conn.lock().await;
        conn.execute(
            r#"
            INSERT INTO recipes (display_name, produced_per_craft, success_rate, critical_success_rate, ingredients_json, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP)
            ON CONFLICT(display_name) DO UPDATE SET
                produced_per_craft = excluded.produced_per_craft,
                success_rate = excluded.success_rate,
                critical_success_rate = excluded.critical_success_rate,
                ingredients_json = excluded.ingredients_json,
                updated_at = CURRENT_TIMESTAMP
            "#,
            params![display_name, produced_per_craft, success_rate, critical_success_rate, ingredients_json],
        )?;
        Ok(())
    }

    /// 전체 레시피 조회 (잔영 및 해연 필터 지원)
    pub async fn get_recipes(&self, pattern: Option<&str>) -> Result<Vec<RecipeRecord>> {
        let conn = self.conn.lock().await;
        let mut list = Vec::new();

        if let Some(p) = pattern {
            let filter = format!("%{}%", p);
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
    }
}
