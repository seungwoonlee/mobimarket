use crate::calculator::CraftCalculator;
use crate::cli::CliClient;
use crate::collector::Collector;
use crate::config::DEFAULT_MONITORED_NAMES;
use crate::store::Database;
use axum::{
    extract::{Query, State},
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub cli: Arc<CliClient>,
    pub db: Database,
    pub collector: Collector,
}

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub name: String,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RecipeQuery {
    pub pattern: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CalculateQuery {
    pub recipe: String,
    pub count: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ItemPriceSummary {
    pub item_name: String,
    pub rarity: Option<String>,
    pub min_unit_price: i64,
    pub in_stock: bool,
    pub change_24h_percent: f64,
    pub scraped_at: String,
}

#[derive(Debug, Deserialize)]
pub struct GroupHistoryQuery {
    pub names: String,
    pub limit: Option<i64>,
}

pub fn create_routes(state: AppState) -> Router {
    Router::new()
        .route("/api/status", get(get_status))
        .route("/api/my_info", get(get_my_info))
        .route("/api/prices", get(get_prices))
        .route("/api/history", get(get_history))
        .route("/api/group_history", get(get_group_history))
        .route("/api/recipes", get(get_recipes))
        .route("/api/calculate", get(get_calculation))
        .route("/api/refresh", post(post_refresh))
        .with_state(state)
}

async fn get_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    match state.cli.check_status().await {
        Ok(s) => {
            let job = state.cli.get_my_info().await.ok().and_then(|info| info.job_name);
            Json(serde_json::json!({
                "connected": s.pipe == "connected",
                "reason": s.reason,
                "job_name": job
            }))
        }
        Err(e) => Json(serde_json::json!({ "connected": false, "error": e, "job_name": null })),
    }
}

async fn get_my_info(State(state): State<AppState>) -> Json<serde_json::Value> {
    match state.cli.get_my_info().await {
        Ok(info) => Json(serde_json::json!({ "success": true, "data": info })),
        Err(e) => Json(serde_json::json!({ "success": false, "error": e })),
    }
}

async fn get_prices(State(state): State<AppState>) -> Json<Vec<ItemPriceSummary>> {
    let mut list = Vec::new();
    for &name in DEFAULT_MONITORED_NAMES {
        if let Ok(Some(latest)) = state.db.get_latest_snapshot(name).await {
            // 과거 기록 1건 조회해서 등락률 모의/계산
            let history = state.db.get_snapshot_history(name, 20).await.unwrap_or_default();
            let change = if history.len() >= 2 {
                let prev = history.first().unwrap().min_unit_price as f64;
                let cur = latest.min_unit_price as f64;
                if prev > 0.0 {
                    (((cur - prev) / prev) * 100.0 * 10.0).round() / 10.0
                } else {
                    0.0
                }
            } else {
                0.0
            };

            list.push(ItemPriceSummary {
                item_name: latest.item_name,
                rarity: latest.rarity,
                min_unit_price: latest.min_unit_price,
                in_stock: latest.in_stock,
                change_24h_percent: change,
                scraped_at: latest.scraped_at,
            });
        }
    }
    Json(list)
}

async fn get_history(
    State(state): State<AppState>,
    Query(query): Query<HistoryQuery>,
) -> Json<Vec<crate::store::SnapshotRecord>> {
    let limit = query.limit.unwrap_or(50);
    let history = state.db.get_snapshot_history(&query.name, limit).await.unwrap_or_default();
    Json(history)
}

#[derive(Debug, Serialize)]
pub struct GroupHistoryItem {
    pub name: String,
    pub history: Vec<crate::store::SnapshotRecord>,
}

async fn get_group_history(
    State(state): State<AppState>,
    Query(query): Query<GroupHistoryQuery>,
) -> Json<Vec<GroupHistoryItem>> {
    let limit = query.limit.unwrap_or(30);
    let names: Vec<&str> = query.names.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    let mut results = Vec::new();

    for name in names {
        let history = state.db.get_snapshot_history(name, limit).await.unwrap_or_default();
        results.push(GroupHistoryItem {
            name: name.to_string(),
            history,
        });
    }

    Json(results)
}

async fn get_recipes(
    State(state): State<AppState>,
    Query(query): Query<RecipeQuery>,
) -> Json<Vec<crate::store::RecipeRecord>> {
    let recipes = state.db.get_recipes(query.pattern.as_deref()).await.unwrap_or_default();
    Json(recipes)
}

async fn get_calculation(
    State(state): State<AppState>,
    Query(query): Query<CalculateQuery>,
) -> Json<serde_json::Value> {
    let count = query.count.unwrap_or(1);
    let calc = CraftCalculator::new(state.db.clone());
    match calc.calculate(&query.recipe, count).await {
        Ok(res) => Json(serde_json::json!({ "success": true, "data": res })),
        Err(e) => Json(serde_json::json!({ "success": false, "error": e })),
    }
}

async fn post_refresh(State(state): State<AppState>) -> Json<serde_json::Value> {
    match state.collector.run_once().await {
        Ok(_) => Json(serde_json::json!({ "success": true })),
        Err(e) => Json(serde_json::json!({ "success": false, "error": e })),
    }
}
