pub mod material_profit;
#[allow(unused_imports)]
pub use material_profit::{MaterialProfitCalculator, MaterialProfitRow};

use crate::cli::models::OwnedItem;
use crate::cli::CraftIngredient;
use crate::config::{format_number, is_zero_cost_material, MARKET_FEE_RATE};
use crate::store::{Database, RecipeRecord};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    pub owned_materials_sell_gross: i64,
    pub owned_materials_sell_net: i64,
    pub all_owned_materials_sell_net: i64,
    pub buy_after_selling_net_cost: Option<i64>,
    pub can_buy_with_used_owned: bool,
    pub can_buy_with_all_owned: bool,
    pub owned_sell_surplus: Option<i64>,
    pub craft_advantage_over_sell_buy: Option<i64>,
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
    pub used_owned_count: i64,
    pub bag_count: i64,
    pub char_storage_count: i64,
    pub account_storage_count: i64,
    pub shortage_count: i64,
    pub unit_min_price: Option<i64>,
    pub total_cost: Option<i64>,
    pub gap_cost: Option<i64>,
    pub owned_sell_gross: Option<i64>,
    pub owned_sell_net: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllRecipeComparisonRow {
    pub display_name: String,
    pub produced_quantity: i64,
    pub product_total_market_price: Option<i64>,
    pub total_material_cost: i64,
    pub user_inventory_gap_cost: i64,
    pub owned_materials_sell_net: i64,
    pub all_owned_materials_sell_net: i64,
    pub buy_after_selling_net_cost: Option<i64>,
    pub can_buy_with_used_owned: bool,
    pub can_buy_with_all_owned: bool,
    pub owned_sell_surplus: Option<i64>,
    pub craft_advantage_over_sell_buy: Option<i64>,
    pub has_missing_prices: bool,
    pub recommendation: String,
    /// 3: 보유재료 매각만으로 완제품 구매 가능, 2: 매각 후 구매 유리, 1: 직접 제작 유리, 0: 동일/시세미수집
    pub status_code: u8,
}

pub struct CraftCalculator {
    db: Database,
}

fn lookup_price(prices: &HashMap<String, i64>, name: &str) -> Option<i64> {
    if let Some(&p) = prices.get(name) {
        return Some(p);
    }
    if name.ends_with("ZZ") {
        prices.get(name.trim_end_matches("ZZ")).copied()
    } else {
        prices.get(&format!("{}ZZ", name)).copied()
    }
}

fn calc_sell_net(gross: i64) -> i64 {
    if gross <= 0 {
        0
    } else {
        ((gross as f64) * (1.0 - MARKET_FEE_RATE)).round() as i64
    }
}

impl CraftCalculator {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// 레시피에 대한 4단 최저가 원가, 보유재료 매각 기회비용(수수료 10% 공제) 및 마진 산출
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

        let price_map = self.db.get_all_latest_price_map().await
            .map_err(|e| format!("시세 맵 조회 실패: {}", e))?;

        Self::calculate_from_price_map(
            &recipe,
            craft_count,
            owned_items,
            has_live_inventory,
            consider_success,
            consider_critical,
            &price_map,
        )
    }

    /// 전체 레시피에 대한 보유 재료 매각(90%) vs 직접 제작 기회비용 일괄 평가 (단일 시세 맵 기반 고속 계산)
    pub async fn evaluate_all_recipes(
        &self,
        recipes: &[RecipeRecord],
        craft_count: i64,
        owned_items: &[OwnedItem],
        has_live_inventory: bool,
        consider_success: bool,
        consider_critical: bool,
    ) -> Result<Vec<AllRecipeComparisonRow>, String> {
        if craft_count <= 0 {
            return Err("제작 횟수는 1 이상이어야 합니다.".to_string());
        }

        let price_map = self.db.get_all_latest_price_map().await
            .map_err(|e| format!("일괄 시세 맵 조회 실패: {}", e))?;

        let mut rows = Vec::with_capacity(recipes.len());
        for recipe in recipes {
            if let Ok(res) = Self::calculate_from_price_map(
                recipe,
                craft_count,
                owned_items,
                has_live_inventory,
                consider_success,
                consider_critical,
                &price_map,
            ) {
                let status_code = if res.has_missing_prices || res.craft_advantage_over_sell_buy.is_none() {
                    0
                } else if res.can_buy_with_used_owned {
                    3
                } else if res.craft_advantage_over_sell_buy.is_some_and(|adv| adv < 0) {
                    2
                } else if res.craft_advantage_over_sell_buy.is_some_and(|adv| adv > 0) {
                    1
                } else {
                    0
                };

                rows.push(AllRecipeComparisonRow {
                    display_name: res.display_name,
                    produced_quantity: res.produced_quantity,
                    product_total_market_price: res.product_total_market_price,
                    total_material_cost: res.total_material_cost,
                    user_inventory_gap_cost: res.user_inventory_gap_cost,
                    owned_materials_sell_net: res.owned_materials_sell_net,
                    all_owned_materials_sell_net: res.all_owned_materials_sell_net,
                    buy_after_selling_net_cost: res.buy_after_selling_net_cost,
                    can_buy_with_used_owned: res.can_buy_with_used_owned,
                    can_buy_with_all_owned: res.can_buy_with_all_owned,
                    owned_sell_surplus: res.owned_sell_surplus,
                    craft_advantage_over_sell_buy: res.craft_advantage_over_sell_buy,
                    has_missing_prices: res.has_missing_prices,
                    recommendation: res.recommendation,
                    status_code,
                });
            }
        }
        Ok(rows)
    }

    /// 단일/배치 공용 순수 계산 엔진
    pub fn calculate_from_price_map(
        recipe: &RecipeRecord,
        craft_count: i64,
        owned_items: &[OwnedItem],
        has_live_inventory: bool,
        consider_success: bool,
        consider_critical: bool,
        prices: &HashMap<String, i64>,
    ) -> Result<CraftCalculationResult, String> {
        if craft_count <= 0 {
            return Err("제작 횟수는 1 이상이어야 합니다.".to_string());
        }

        let raw_ingredients: Vec<CraftIngredient> = serde_json::from_str(&recipe.ingredients_json)
            .map_err(|e| format!("재료 JSON 파싱 실패: {}", e))?;

        let prod_per_craft = recipe.produced_per_craft.max(1);
        let produced_quantity = prod_per_craft * craft_count;

        // 완제품 최저 매물가 조회
        let product_unit_min_price = lookup_price(prices, &recipe.display_name);
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
        let mut owned_materials_sell_gross: i64 = 0;
        let mut owned_materials_sell_net: i64 = 0;
        let mut all_owned_materials_sell_net: i64 = 0;
        let mut missing_price_items: Vec<String> = Vec::new();
        let mut breakdown = Vec::new();

        for ing in raw_ingredients {
            let total_req_base = ing.required * craft_count;
            let expected_req = if consider_success || consider_critical {
                ((ing.required as f64) * expected_attempts).ceil() as i64
            } else {
                total_req_base
            };

            let ing_norm = ing.display_name.trim_end_matches("ZZ");
            let mut bag_count: i64 = 0;
            let mut char_storage_count: i64 = 0;
            let mut account_storage_count: i64 = 0;

            for item in owned_items {
                if item.is_locked || item.display_name.trim_end_matches("ZZ") != ing_norm {
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
            let total_owned = if has_live_inventory {
                live_sum
            } else {
                live_sum.max(ing.owned)
            };
            let used_owned = total_owned.min(expected_req);
            let shortage = (expected_req - total_owned).max(0);

            let unit_price = if is_zero_cost_material(&ing.display_name) {
                Some(0)
            } else {
                let price = lookup_price(prices, &ing.display_name);
                if price.is_none() {
                    missing_price_items.push(ing.display_name.clone());
                }
                price
            };

            let ing_total_cost = unit_price.map(|p| p * expected_req);
            let ing_gap_cost = unit_price.map(|p| p * shortage);
            let ing_sell_gross = unit_price.map(|p| p * used_owned);
            let ing_sell_net = ing_sell_gross.map(calc_sell_net);
            let ing_all_owned_net = unit_price.map(|p| calc_sell_net(p * total_owned));

            if let Some(cost) = ing_total_cost {
                total_material_cost += cost;
            }
            if let Some(gap) = ing_gap_cost {
                user_inventory_gap_cost += gap;
            }
            if let Some(gross) = ing_sell_gross {
                owned_materials_sell_gross += gross;
            }
            if let Some(net) = ing_sell_net {
                owned_materials_sell_net += net;
            }
            if let Some(all_net) = ing_all_owned_net {
                all_owned_materials_sell_net += all_net;
            }

            breakdown.push(IngredientCostBreakdown {
                display_name: ing.display_name,
                required_per_craft: ing.required,
                total_required: total_req_base,
                expected_required: expected_req,
                owned_count: total_owned,
                used_owned_count: used_owned,
                bag_count,
                char_storage_count,
                account_storage_count,
                shortage_count: shortage,
                unit_min_price: unit_price,
                total_cost: ing_total_cost,
                gap_cost: ing_gap_cost,
                owned_sell_gross: ing_sell_gross,
                owned_sell_net: ing_sell_net,
            });
        }

        let has_missing_prices = !missing_price_items.is_empty();
        let valid_product_price = product_total_market_price.filter(|&p| p > 0);

        let (
            buy_after_selling_net_cost,
            can_buy_with_used_owned,
            can_buy_with_all_owned,
            owned_sell_surplus,
            craft_advantage_over_sell_buy,
            saving_amount,
            net_profit,
            roi_percent,
            recommendation,
        ) = if has_missing_prices {
            (
                None,
                false,
                false,
                None,
                None,
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

            match valid_product_price {
                Some(p_market) => {
                    let buy_after_sell = p_market - owned_materials_sell_net;
                    let can_buy_used = owned_materials_sell_net >= p_market;
                    let can_buy_all = all_owned_materials_sell_net >= p_market;
                    let surplus = owned_materials_sell_net - p_market;
                    let advantage = buy_after_sell - user_inventory_gap_cost;

                    let ratio = (advantage.abs() as f64) / (p_market as f64);
                    let net_sell = profit.unwrap_or(0);
                    let sell_hint = if net_sell > 0 {
                        format!(" / 완제품 판매 시 순익(수수료 10% 후) +{} 데카", format_number(net_sell))
                    } else {
                        format!(" / 완제품 판매 시 {} 데카 손해(수수료 10%)", format_number(-net_sell))
                    };

                    let rec = if advantage > 0 {
                        let base_label = if owned_materials_sell_net > 0 {
                            "보유 재료로 직접 제작"
                        } else {
                            "직접 제작"
                        };
                        let cmp_target = if owned_materials_sell_net > 0 {
                            "재료 매각 후 구매 대비"
                        } else {
                            "완제품 대비"
                        };
                        if ratio >= 0.10 {
                            format!(
                                "{} 강력 추천 ({} {} 데카 / {:.1}% 유리{})",
                                base_label,
                                cmp_target,
                                format_number(advantage),
                                ratio * 100.0,
                                sell_hint
                            )
                        } else {
                            format!(
                                "{} 추천 ({} {} 데카 유리{})",
                                base_label,
                                cmp_target,
                                format_number(advantage),
                                sell_hint
                            )
                        }
                    } else if advantage == 0 {
                        if can_buy_used {
                            format!("직접 제작 / 보유 재료 매각 구매 동일 (보유 재료 매각만으로 구매 가능{})", sell_hint)
                        } else {
                            format!("직접 제작 / 재료 매각 후 완제품 구매 동일{}", sell_hint)
                        }
                    } else {
                        let buy_gain = -advantage;
                        if can_buy_used {
                            format!(
                                "보유 재료 매각 후 마켓 완제품 구매 강력 추천 (보유 재료만 팔아도 구매 가능, +{} 데카 잉여 / 제작 대비 {} 데카 유리)",
                                format_number(surplus),
                                format_number(buy_gain)
                            )
                        } else if owned_materials_sell_net > 0 {
                            if ratio >= 0.10 {
                                format!(
                                    "보유 재료 매각 후 마켓 완제품 구매 강력 추천 (직접 제작 대비 {} 데카 / {:.1}% 저렴)",
                                    format_number(buy_gain),
                                    ratio * 100.0
                                )
                            } else {
                                format!(
                                    "보유 재료 매각 후 마켓 완제품 구매 추천 (직접 제작 대비 {} 데카 저렴)",
                                    format_number(buy_gain)
                                )
                            }
                        } else if ratio >= 0.10 {
                            format!(
                                "마켓 완제품 구매 강력 추천 (직접 제작 시 {} 데카 손해)",
                                format_number(buy_gain)
                            )
                        } else {
                            format!(
                                "마켓 완제품 구매 추천 (직접 제작 시 {} 데카 손해)",
                                format_number(buy_gain)
                            )
                        }
                    };

                    (
                        Some(buy_after_sell),
                        can_buy_used,
                        can_buy_all,
                        Some(surplus),
                        Some(advantage),
                        saving,
                        profit,
                        roi,
                        rec,
                    )
                }
                None => (
                    None,
                    false,
                    false,
                    None,
                    None,
                    saving,
                    profit,
                    roi,
                    "완제품 마켓 매물 없음 (제작만 가능)".to_string(),
                ),
            }
        };

        Ok(CraftCalculationResult {
            display_name: recipe.display_name.clone(),
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
            owned_materials_sell_gross,
            owned_materials_sell_net,
            all_owned_materials_sell_net,
            buy_after_selling_net_cost,
            can_buy_with_used_owned,
            can_buy_with_all_owned,
            owned_sell_surplus,
            craft_advantage_over_sell_buy,
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
