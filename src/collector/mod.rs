use crate::cli::CliClient;
use crate::store::Database;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{info, warn};

#[derive(Clone)]
pub struct Collector {
    cli: Arc<CliClient>,
    db: Database,
    pub interval_secs: Arc<AtomicU64>,
    pub remaining_secs: Arc<AtomicU64>,
    pub trigger_now: Arc<AtomicBool>,
    pub is_collecting: Arc<AtomicBool>,
}

impl Collector {
    pub fn new(cli: Arc<CliClient>, db: Database, default_interval_secs: u64) -> Self {
        Self {
            cli,
            db,
            interval_secs: Arc::new(AtomicU64::new(default_interval_secs)),
            remaining_secs: Arc::new(AtomicU64::new(default_interval_secs)),
            trigger_now: Arc::new(AtomicBool::new(false)),
            is_collecting: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 수동 즉시 수집 트리거
    pub fn request_immediate_run(&self) {
        self.trigger_now.store(true, Ordering::Relaxed);
    }

    /// 레시피 및 시세 1회 일괄 수집
    pub async fn run_once(&self) -> Result<(), String> {
        self.is_collecting.store(true, Ordering::Relaxed);

        // 1. 상태 점검
        let status = match self.cli.check_status().await {
            Ok(s) => s,
            Err(e) => {
                self.is_collecting.store(false, Ordering::Relaxed);
                return Err(format!("CLI 상태 확인 실패: {}", e));
            }
        };

        if status.pipe != "connected" {
            self.is_collecting.store(false, Ordering::Relaxed);
            warn!("게임 CLI가 연결되어 있지 않습니다: {:?}", status.reason);
            return Err("게임 클라이언트 미연동".to_string());
        }

        // 2. 레시피 동기화 (잔영 / 해연 중심)
        if let Ok(craft_data) = self.cli.get_craftable_items().await {
            info!("총 {}개의 제작 레시피 수신 완료. 잔영/해연 동기화 중...", craft_data.items.len());
            for item in craft_data.items {
                if item.display_name.contains("잔영") || item.display_name.contains("해연") {
                    let ingredients_json = serde_json::to_string(&item.ingredients).unwrap_or_default();
                    let _ = self.db.upsert_recipe(
                        &item.display_name,
                        item.produced_per_craft,
                        item.success_rate,
                        item.critical_success_rate,
                        &ingredients_json,
                    ).await;
                }
            }
        }

        // 3. 모니터링 품목 목록 (사용자가 추가한 품목 + 기본 품목)
        let mut names: Vec<String> = self.db.get_monitored_items().await.unwrap_or_default();
        if names.is_empty() {
            names = crate::config::DEFAULT_MONITORED_NAMES.iter().map(|s| s.to_string()).collect();
        }

        // 잔영/해연 레시피의 하위 재료들 및 완제품 이름들도 모니터링 대상에 포함
        if let Ok(recipes) = self.db.get_recipes(None).await {
            for r in recipes {
                if !names.contains(&r.display_name) {
                    names.push(r.display_name.clone());
                }
                if let Ok(raw_ings) = serde_json::from_str::<Vec<crate::cli::CraftIngredient>>(&r.ingredients_json) {
                    for ing in raw_ings {
                        if !names.contains(&ing.display_name) {
                            names.push(ing.display_name);
                        }
                    }
                }
            }
        }

        info!("총 {}개 품목 최저가 시세 수집 시작...", names.len());
        for chunk in names.chunks(20) {
            match self.cli.get_market_prices(chunk).await {
                Ok(res) => {
                    if let Some(results) = res.results {
                        for q in results {
                            for row in q.rows {
                                if let Some(min_price) = row.min_unit_price {
                                    let _ = self.db.insert_snapshot(
                                        &row.display_name,
                                        row.rarity.as_deref(),
                                        row.star,
                                        row.season.as_deref(),
                                        min_price,
                                        row.in_stock,
                                    ).await;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("청크 시세 수집 실패: {}", e);
                }
            }
            sleep(Duration::from_millis(400)).await;
        }

        info!("최저가 시세 동기화 완료!");
        self.is_collecting.store(false, Ordering::Relaxed);
        Ok(())
    }

    /// 백그라운드 1초 카운트다운 반응형 루프
    pub fn start_background_loop(self) {
        tokio::spawn(async move {
            let mut elapsed: u64 = 0;
            loop {
                sleep(Duration::from_secs(1)).await;
                elapsed += 1;

                let interval = self.interval_secs.load(Ordering::Relaxed);
                let triggered = self.trigger_now.swap(false, Ordering::Relaxed);

                if triggered || elapsed >= interval {
                    elapsed = 0;
                    self.remaining_secs.store(interval, Ordering::Relaxed);
                    let _ = self.run_once().await;
                } else {
                    let rem = if interval > elapsed { interval - elapsed } else { 0 };
                    self.remaining_secs.store(rem, Ordering::Relaxed);
                }
            }
        });
    }
}
