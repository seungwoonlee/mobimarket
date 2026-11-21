pub mod material_profit;
#[allow(unused_imports)]
pub use material_profit::{MaterialProfitCalculator, MaterialProfitRow};

use crate::cli::models::OwnedItem;
use crate::cli::CraftIngredient;
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
    /// - owned_items: get_items로 조회된 내가방 + 내창고 + 공용창고 전체 아이템 목록
    /// - consider_success: 성공률 감안 여부 (기본 true: 90%면 1.11회 시도 필요)
    /// - consider_critical: 대성공 감안 여부 (기본 false: 대성공 시 2개 산출 반영)
    pub async fn calculate(
        &self,
        recipe_name: &str,
        craft_count: i64,
        owned_items: &[OwnedItem],
        consider_success: bool,
        consider_critical: bool,
    ) -> Result<CraftCalculationResult, String> {
        let recipes = self.db.get_recipes(Some(recipe_name)).await
            .map_err(|e| format!("레시피 조회 실패: {}", e))?;

        let recipe = recipes.into_iter().find(|r| r.display_name == recipe_name)
            .ok_or_else(|| format!("레시피를 찾을 수 없습니다: {}", recipe_name))?;

        let raw_ingredients: Vec<CraftIngredient> = serde_json::from_str(&recipe.ingredients_json)
            .map_err(|e| format!("재료 JSON 파싱 실패: {}", e))?;

        // 완제품 최저 매물가 조회
        let product_snapshot = self.db.get_latest_snapshot(&recipe.display_name).await
            .map_err(|e| format!("완제품 시세 조회 실패: {}", e))?;

        let product_unit_min_price = product_snapshot.map(|s| s.min_unit_price);
        let product_total_market_price = product_unit_min_price.map(|p| p * craft_count * recipe.produced_per_craft);

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

        // 2. 1회 시도당 완제품 산출 기대량 E 계산
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

        // 대성공은 성공 시 추가 1개(+1) 산출: E = p_s * (1.0 + p_c)
        let expected_yield_per_craft = (p_s * (1.0 + p_c)).max(0.01);
        let expected_attempts = (craft_count as f64) / expected_yield_per_craft;

        let mut total_material_cost: i64 = 0;
        let mut user_inventory_gap_cost: i64 = 0;
        let mut breakdown = Vec::new();

        for ing in raw_ingredients {
            let total_req_base = ing.required * craft_count;
            // 게임 내 아이템은 0.1개 단위로 살 수 없으므로 올림(ceil) 정수 처리
            let expected_req = if consider_success || consider_critical {
                ((ing.required as f64) * expected_attempts).ceil() as i64
            } else {
                total_req_base
            };

            // 내가방 + 내창고 + 공용창고 수량 각각 및 총합 계산
            let mut bag_count: i64 = 0;
            let mut char_storage_count: i64 = 0;
            let mut account_storage_count: i64 = 0;

            for item in owned_items {
                if item.display_name == ing.display_name {
                    match item.location.as_str() {
                        "inventory" => bag_count += item.count,
                        "character_storage" => char_storage_count += item.count,
                        "account_storage" => account_storage_count += item.count,
                        _ => bag_count += item.count,
                    }
                }
            }

            // get_items에 없더라도 레시피 내 ing.owned가 있으면 fallback
            let total_owned = (bag_count + char_storage_count + account_storage_count).max(ing.owned);
            let shortage = (expected_req - total_owned).max(0);

            let ing_snapshot = self.db.get_latest_snapshot(&ing.display_name).await
                .map_err(|e| format!("재료 시세 조회 실패({}): {}", ing.display_name, e))?;

            let unit_price = ing_snapshot.map(|s| s.min_unit_price);
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

        let saving_amount = product_total_market_price.map(|p| p - total_material_cost);
        let net_profit = product_total_market_price.map(|p| ((p as f64 * 0.95) - total_material_cost as f64).round() as i64);
        let roi_percent = if total_material_cost > 0 {
            net_profit.map(|np| ((np as f64 / total_material_cost as f64) * 100.0 * 10.0).round() / 10.0)
        } else {
            None
        };

        // 추천 판정 알고리즘 (사용자 요구사항: 10% 이상 차이 시에만 "강력 추천" 적용)
        let recommendation = match product_total_market_price {
            Some(p_market) if p_market > 0 => {
                let saving = p_market - total_material_cost;
                let ratio = (saving.abs() as f64) / (p_market as f64);
                if saving > 0 {
                    if ratio >= 0.10 {
                        format!("직접 제작 강력 추천 (완제품 대비 {} 데카 / {:.1}% 절약)", format_num(saving), ratio * 100.0)
                    } else {
                        format!("직접 제작 추천 (완제품 대비 {} 데카 절약)", format_num(saving))
                    }
                } else if saving == 0 {
                    "제작/구매 동일 (마진 없음)".to_string()
                } else {
                    let loss = -saving;
                    if ratio >= 0.10 {
                        format!("마켓 완제품 구매 강력 추천 (직접 제작 시 {} 데카 손해)", format_num(loss))
                    } else {
                        format!("마켓 완제품 구매 추천 (직접 제작 시 {} 데카 손해)", format_num(loss))
                    }
                }
            }
            _ => "완제품 마켓 매물 없음 (제작만 가능)".to_string(),
        };

        Ok(CraftCalculationResult {
            display_name: recipe.display_name,
            craft_count,
            produced_quantity: recipe.produced_per_craft * craft_count,
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
            saving_amount,
            net_profit,
            roi_percent,
            recommendation,
            ingredients: breakdown,
        })
    }
}

fn format_num(n: i64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    for (i, c) in chars.into_iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result
}
