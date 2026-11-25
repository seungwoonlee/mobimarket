pub mod material_profit;
#[allow(unused_imports)]
pub use material_profit::{MaterialProfitCalculator, MaterialProfitRow};

use crate::cli::models::OwnedItem;
use crate::cli::CraftIngredient;
use crate::config::{format_number, is_zero_cost_material, MARKET_FEE_RATE};
use crate::store::Database;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CraftCalculationResult {
    pub display_name: String,
    pub craft_count: i64,
    pub produced_quantity: i64,
    pub success_rate: f64,
    pub critical_success_rate: f64,
    pub normalized_success_rate: f64,
    pub normalized_critical_success_rate: f64,
    pub consider_success: bool,
    pub consider_critical: bool,
    pub expected_attempts: f64,
    pub product_unit_min_price: Option<i64>,
    pub product_total_market_price: Option<i64>,
    pub total_material_cost: i64,
    pub user_inventory_gap_cost: i64,
    pub has_missing_prices: bool,
    pub missing_price_items: Vec<String>,
    pub saving_amount: Option<i64>,
    pub net_profit: Option<i64>,
    pub roi_percent: Option<f64>,
    pub recommendation: String,
    pub ingredients: Vec<IngredientCostBreakdown>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngredientCostBreakdown {
    pub display_name: String,
    pub required_per_craft: i64,
    pub total_required: i64,
    pub expected_required: i64,
    pub owned_count: i64,
    pub bag_count: i64,
    pub char_storage_count: i64,
    pub account_storage_count: i64,
    pub shortage_count: i64,
    pub unit_min_price: Option<i64>,
    pub total_cost: Option<i64>,
    pub gap_cost: Option<i64>,
}

pub struct CraftCalculator {
    db: Database,
}

impl CraftCalculator {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// 레시피에 대한 3단 최저가 원가 및 마진 산출 (성공률/대성공률 기대값 정수화 반영)
    /// - craft_count: 제작 성공 기준 목표 횟수 (1 이상)
    /// - owned_items: get_items로 조회된 내가방 + 내창고 + 공용창고 전체 아이템 목록 (잠금 아이템 자동 제외)
    /// - has_live_inventory: 실시간 인벤토리 연동 여부 (true일 때는 빈 목록이어도 과거 ing.owned fallback 미사용)
    /// - consider_success: 성공률 감안 여부 (기본 true: 90%면 1.11회 시도 필요)
    /// - consider_critical: 대성공 감안 여부 (기본 false: 대성공 시 +1개 추가 산출 반영)
    pub async fn calculate(
        &self,
        recipe_name: &str,
        craft_count: i64,
        owned_items: &[OwnedItem],
        has_live_inventory: bool,
        consider_success: bool,
        consider_critical: bool,
    ) -> Result<CraftCalculationResult, String> {
        if craft_count <= 0 {
            return Err("제작 횟수는 1 이상이어야 합니다.".to_string());
        }

        let recipes = self.db.get_recipes(Some(recipe_name)).await
            .map_err(|e| format!("레시피 조회 실패: {}", e))?;

        let recipe = recipes.into_iter().find(|r| r.display_name == recipe_name)
            .ok_or_else(|| format!("레시피를 찾을 수 없습니다: {}", recipe_name))?;

        let raw_ingredients: Vec<CraftIngredient> = serde_json::from_str(&recipe.ingredients_json)
            .map_err(|e| format!("재료 JSON 파싱 실패: {}", e))?;

        let prod_per_craft = recipe.produced_per_craft.max(1);
        let produced_quantity = prod_per_craft * craft_count;

        // 완제품 최저 매물가 조회
        let product_snapshot = self.db.get_latest_snapshot(&recipe.display_name).await
            .map_err(|e| format!("완제품 시세 조회 실패: {}", e))?;

        let product_unit_min_price = product_snapshot.map(|s| s.min_unit_price);
        let product_total_market_price = product_unit_min_price.map(|p| p * produced_quantity);

        // 1. 성공률 / 대성공률 정규화 (CLI는 100.0, 90.0, 12.0 등 백분율로 반환)
        let norm_succ = if recipe.success_rate > 0.0 && recipe.success_rate <= 1.0 {
            recipe.success_rate * 100.0
        } else {
            recipe.success_rate
        };
        let norm_crit = if recipe.critical_success_rate > 0.0 && recipe.critical_success_rate <= 1.0 {
            recipe.critical_success_rate * 100.0
        } else {
            recipe.critical_success_rate
        };

        // 2. 1회 시도당 완제품 산출 기대량 E 계산 (대성공은 성공 시 +1개 추가 산출)
        let p_s = if consider_success {
            (norm_succ / 100.0).clamp(0.01, 1.0)
        } else {
            1.0
        };

        let p_c = if consider_critical {
            (norm_crit / 100.0).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let expected_yield_per_craft = (p_s * (prod_per_craft as f64 + p_c)).max(0.01);
        let expected_attempts = (produced_quantity as f64) / expected_yield_per_craft;

        let mut total_material_cost: i64 = 0;
        let mut user_inventory_gap_cost: i64 = 0;
        let mut missing_price_items: Vec<String> = Vec::new();
        let mut breakdown = Vec::new();

        for ing in raw_ingredients {
            let total_req_base = ing.required * craft_count;
            // 게임 내 아이템은 0.1개 단위로 살 수 없으므로 올림(ceil) 정수 처리
            let expected_req = if consider_success || consider_critical {
                ((ing.required as f64) * expected_attempts).ceil() as i64
            } else {
                total_req_base
            };

            // 내가방 + 내창고 + 공용창고 수량 각각 및 총합 계산 (잠금 아이템은 제작 소모 불가이므로 제외)
            let mut bag_count: i64 = 0;
            let mut char_storage_count: i64 = 0;
            let mut account_storage_count: i64 = 0;

            for item in owned_items {
                if item.is_locked || item.display_name != ing.display_name {
                    continue;
                }
                match item.location.as_str() {
                    "inventory" => bag_count += item.count,
                    "character_storage" => char_storage_count += item.count,
                    "account_storage" => account_storage_count += item.count,
                    _ => bag_count += item.count,
                }
            }

            let live_sum = bag_count + char_storage_count + account_storage_count;
            // 실시간 인벤토리 미연동 상태에서만 레시피 동기화 당시 ing.owned를 fallback 사용
            let total_owned = if has_live_inventory {
                live_sum
            } else {
                live_sum.max(ing.owned)
            };
            let shortage = (expected_req - total_owned).max(0);

            let unit_price = if is_zero_cost_material(&ing.display_name) {
                Some(0)
            } else {
                let ing_snapshot = self.db.get_latest_snapshot(&ing.display_name).await
                    .map_err(|e| format!("재료 시세 조회 실패({}): {}", ing.display_name, e))?;

                let price = ing_snapshot.map(|s| s.min_unit_price);
                if price.is_none() {
                    missing_price_items.push(ing.display_name.clone());
                }
                price
            };

            let ing_total_cost = unit_price.map(|p| p * expected_req);
            let ing_gap_cost = unit_price.map(|p| p * shortage);

            if let Some(cost) = ing_total_cost {
                total_material_cost += cost;
            }
            if let Some(gap) = ing_gap_cost {
                user_inventory_gap_cost += gap;
            }

            breakdown.push(IngredientCostBreakdown {
                display_name: ing.display_name,
                required_per_craft: ing.required,
                total_required: total_req_base,
                expected_required: expected_req,
                owned_count: total_owned,
                bag_count,
                char_storage_count,
                account_storage_count,
                shortage_count: shortage,
                unit_min_price: unit_price,
                total_cost: ing_total_cost,
                gap_cost: ing_gap_cost,
            });
        }

        let has_missing_prices = !missing_price_items.is_empty();

        let (saving_amount, net_profit, roi_percent, recommendation) = if has_missing_prices {
            (
                None,
                None,
                None,
                format!(
                    "재료 시세 일부 미수집 ({}) — 정확한 원가 비교 불가",
                    missing_price_items.join(", ")
                ),
            )
        } else {
            let saving = product_total_market_price.map(|p| p - total_material_cost);
            let profit = product_total_market_price
                .map(|p| ((p as f64 * (1.0 - MARKET_FEE_RATE)) - total_material_cost as f64).round() as i64);
            let roi = if total_material_cost > 0 {
                profit.map(|np| ((np as f64 / total_material_cost as f64) * 100.0 * 10.0).round() / 10.0)
            } else {
                None
            };

            // 추천 판정: 자가 사용 기준 절감액 + 10% 거래소 수수료 반영 판매 차익 안내
            let rec = match product_total_market_price {
                Some(p_market) if p_market > 0 => {
                    let s = p_market - total_material_cost;
                    let ratio = (s.abs() as f64) / (p_market as f64);
                    let net_sell = profit.unwrap_or(0);
                    let sell_hint = if net_sell > 0 {
                        format!(" / 판매 순익(수수료 10% 후) +{} 데카", format_number(net_sell))
                    } else {
                        format!(" / 판매 시 {} 데카 손해(수수료 10%)", format_number(-net_sell))
                    };

                    if s > 0 {
                        if ratio >= 0.10 {
                            format!(
                                "직접 제작 강력 추천 (완제품 대비 {} 데카 / {:.1}% 절약{})",
                                format_number(s),
                                ratio * 100.0,
                                sell_hint
                            )
                        } else {
                            format!(
                                "직접 제작 추천 (완제품 대비 {} 데카 절약{})",
                                format_number(s),
                                sell_hint
                            )
                        }
                    } else if s == 0 {
                        format!("제작/구매 동일 (마진 없음{})", sell_hint)
                    } else {
                        let loss = -s;
                        if ratio >= 0.10 {
                            format!("마켓 완제품 구매 강력 추천 (직접 제작 시 {} 데카 손해)", format_number(loss))
                        } else {
                            format!("마켓 완제품 구매 추천 (직접 제작 시 {} 데카 손해)", format_number(loss))
                        }
                    }
                }
                _ => "완제품 마켓 매물 없음 (제작만 가능)".to_string(),
            };

            (saving, profit, roi, rec)
        };

        Ok(CraftCalculationResult {
            display_name: recipe.display_name,
            craft_count,
            produced_quantity,
            success_rate: recipe.success_rate,
            critical_success_rate: recipe.critical_success_rate,
            normalized_success_rate: norm_succ,
            normalized_critical_success_rate: norm_crit,
            consider_success,
            consider_critical,
            expected_attempts,
            product_unit_min_price,
            product_total_market_price,
            total_material_cost,
            user_inventory_gap_cost,
            has_missing_prices,
            missing_price_items,
            saving_amount,
            net_profit,
            roi_percent,
            recommendation,
            ingredients: breakdown,
        })
    }
}
