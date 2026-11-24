use crate::cli::CliClient;
use crate::store::Database;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{info, warn};

/// run_once() 종료/취소/패닉 시에도 is_collecting 플래그를 반드시 해제하는 RAII 가드
struct CollectingGuard {
    flag: Arc<AtomicBool>,
}

impl Drop for CollectingGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

#[derive(Clone)]
pub struct Collector {
    cli: Arc<CliClient>,
    db: Database,
    pub interval_secs: Arc<AtomicU64>,
    pub remaining_secs: Arc<AtomicU64>,
    pub trigger_now: Arc<AtomicBool>,
    pub is_collecting: Arc<AtomicBool>,
    pub collection_epoch: Arc<AtomicU64>,
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
            collection_epoch: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 수동 즉시 수집 트리거
    pub fn request_immediate_run(&self) {
        self.trigger_now.store(true, Ordering::Release);
    }

    /// 레시피 및 시세 1회 일괄 수집 (동시 중복 실행 방지 및 RAII 상태 보호)
    pub async fn run_once(&self) -> Result<(), String> {
        if self
            .is_collecting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            info!("이미 시세 수집이 진행 중이므로 중복 실행을 건너뜁니다.");
            return Ok(());
        }
        let _guard = CollectingGuard {
            flag: Arc::clone(&self.is_collecting),
        };

        // 1. 상태 점검
        let status = self
            .cli
            .check_status()
            .await
            .map_err(|e| format!("CLI 상태 확인 실패: {}", e))?;

        if status.pipe != "connected" {
            warn!("게임 CLI가 연결되어 있지 않습니다: {:?}", status.reason);
            return Err("게임 클라이언트 미연동".to_string());
        }

        // 2. 레시피 동기화 (잔영 / 해연 중심) 및 재료 품목 자동 모니터링 등록
        match self.cli.get_craftable_items().await {
            Ok(craft_data) => {
                info!(
                    "총 {}개의 제작 레시피 수신 완료. 잔영/해연 동기화 중...",
                    craft_data.items.len()
                );
                for item in craft_data.items {
                    if item.display_name.contains("잔영") || item.display_name.contains("해연") {
                        let ingredients_json =
                            serde_json::to_string(&item.ingredients).unwrap_or_default();
                        if let Err(e) = self
                            .db
                            .upsert_recipe(
                                &item.display_name,
                                item.produced_per_craft,
                                item.success_rate,
                                item.critical_success_rate,
                                &ingredients_json,
                            )
                            .await
                        {
                            warn!("레시피 저장 실패({}): {}", item.display_name, e);
                        }

                        // 레시피의 각 하위 재료를 monitored_items에 '재료' 그룹으로 자동 영구 등록
                        for ing in &item.ingredients {
                            if let Err(e) = self
                                .db
                                .add_monitored_item_ignore(&ing.display_name, "재료")
                                .await
                            {
                                warn!("재료 모니터링 자동 등록 실패({}): {}", ing.display_name, e);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                warn!("레시피 목록 조회 실패: {}", e);
            }
        }

        // 3. 모니터링 품목 목록 (사용자가 추가한 품목 + 기본 품목)
        let mut names: Vec<String> = self.db.get_monitored_items().await.unwrap_or_else(|e| {
            warn!("모니터링 품목 조회 실패: {}", e);
            Vec::new()
        });
        if names.is_empty() {
            names = crate::config::DEFAULT_MONITORED_NAMES
                .iter()
                .map(|s| s.to_string())
                .collect();
        }

        // 잔영/해연 레시피의 하위 재료들 및 완제품 이름들도 모니터링 대상에 포함
        if let Ok(recipes) = self.db.get_recipes(None).await {
            for r in recipes {
                if !names.contains(&r.display_name) {
                    names.push(r.display_name.clone());
                }
                if let Ok(raw_ings) =
                    serde_json::from_str::<Vec<crate::cli::CraftIngredient>>(&r.ingredients_json)
                {
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
                                    if let Err(e) = self
                                        .db
                                        .insert_snapshot(
                                            &row.display_name,
                                            row.rarity.as_deref(),
                                            row.star,
                                            row.season.as_deref(),
                                            min_price,
                                            row.in_stock,
                                        )
                                        .await
                                    {
                                        warn!("스냅샷 저장 실패({}): {}", row.display_name, e);
                                    }
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

        self.collection_epoch.fetch_add(1, Ordering::Release);
        info!("최저가 시세 동기화 완료!");
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
                let triggered = self.trigger_now.swap(false, Ordering::AcqRel);

                if triggered || elapsed >= interval {
                    elapsed = 0;
                    self.remaining_secs.store(interval, Ordering::Relaxed);
                    let _ = self.run_once().await;
                } else {
                    let rem = interval.saturating_sub(elapsed);
                    self.remaining_secs.store(rem, Ordering::Relaxed);
                }
            }
        });
    }
}
