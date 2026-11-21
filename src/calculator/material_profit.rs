use crate::config::{MaterialRecipeDef, MATERIAL_RECIPES};
use std::collections::HashMap;

/// 가공 재료 데카벌이 분석 결과 행
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct MaterialProfitRow {
    pub name: String,
    pub category: String,
    pub market_price: Option<i64>,
    pub material_cost: Option<i64>,
    pub produced_per_craft: i64,
    pub duration_secs: u32,
    pub net_profit_per_craft: Option<i64>,
    pub net_profit_per_unit: Option<i64>,
    pub margin_ratio: Option<f64>,
    pub hourly_profit: Option<i64>,
    pub status_label: &'static str,
    pub status_code: u8, // 4: 꿀벌이, 3: 쏠쏠함, 2: 박리다매, 1: 적자주의, 0: 시세대기
}

/// 가공 재료 데카벌이 계산기
pub struct MaterialProfitCalculator;

impl MaterialProfitCalculator {
    /// 현재 최신 시세 맵과 커스텀 소요시간 오버라이드를 반영하여 분석 수행
    pub fn calculate(
        prices: &HashMap<String, i64>,
        custom_durations: &HashMap<String, u32>,
    ) -> Vec<MaterialProfitRow> {
        let mut results = Vec::new();

        for def in MATERIAL_RECIPES {
            let row = Self::calculate_single(def, prices, custom_durations);
            results.push(row);
        }

        // 시간당 순이익 내림차순 정렬 (시세 없는 항목은 맨 뒤로)
        results.sort_by(|a, b| {
            let profit_b = b.hourly_profit.unwrap_or(i64::MIN);
            let profit_a = a.hourly_profit.unwrap_or(i64::MIN);
            profit_b.cmp(&profit_a)
        });

        results
    }

    /// 단일 품목 분석
    pub fn calculate_single(
        def: &MaterialRecipeDef,
        prices: &HashMap<String, i64>,
        custom_durations: &HashMap<String, u32>,
    ) -> MaterialProfitRow {
        let duration_secs = custom_durations
            .get(def.name)
            .cloned()
            .unwrap_or(def.default_duration_secs)
            .max(1);

        // 1. 완제품 거래소 판매 최저가
        let market_price = prices.get(def.name).cloned();

        // 2. 하위 원자재 총비용 산출 (모든 하위 재료의 시세가 존재해야 함)
        let mut material_cost_sum = 0i64;
        let mut all_ingredients_priced = true;

        for (ing_name, ing_qty) in def.ingredients {
            if let Some(ing_price) = prices.get(*ing_name) {
                material_cost_sum += ing_price * ing_qty;
            } else {
                all_ingredients_priced = false;
                break;
            }
        }

        let material_cost = if all_ingredients_priced {
            Some(material_cost_sum)
        } else {
            None
        };

        // 3. 거래소 수수료 10% 차감 후 순마진 계산
        let (net_profit_per_craft, net_profit_per_unit, margin_ratio, hourly_profit, status_label, status_code) =
            if let (Some(m_price), Some(m_cost)) = (market_price, material_cost) {
                // 실수령액: 판매가 * 1회 생산량 * 90%
                let revenue = (m_price as f64 * def.produced_per_craft as f64 * 0.90).round() as i64;
                let net_craft = revenue - m_cost;
                let net_unit = net_craft / def.produced_per_craft.max(1);
                let margin = if m_cost > 0 {
                    (net_craft as f64 / m_cost as f64) * 100.0
                } else {
                    0.0
                };

                // 시간당 생산 횟수 (3600초 / 1회 소요시간)
                let per_hour_count = 3600.0 / duration_secs as f64;
                let hourly = (per_hour_count * net_craft as f64).round() as i64;

                // 상태 판정
                let (label, code) = if hourly >= 30_000 {
                    ("🔥 꿀벌이 (시급 3만+)", 4)
                } else if hourly >= 10_000 {
                    ("💰 쏠쏠함 (시급 1만+)", 3)
                } else if hourly > 0 {
                    ("⚖️ 박리다매 (소소한 수익)", 2)
                } else {
                    ("⚠️ 적자 주의 (제작 손해!)", 1)
                };

                (Some(net_craft), Some(net_unit), Some(margin), Some(hourly), label, code)
            } else {
                ("⚪ 시세 대기 중", 0, None, None, None, None)
                    .map_empty()
            };

        MaterialProfitRow {
            name: def.name.to_string(),
            category: def.category.to_string(),
            market_price,
            material_cost,
            produced_per_craft: def.produced_per_craft,
            duration_secs,
            net_profit_per_craft,
            net_profit_per_unit,
            margin_ratio,
            hourly_profit,
            status_label,
            status_code,
        }
    }
}

trait EmptyTupleHelper {
    fn map_empty(self) -> (Option<i64>, Option<i64>, Option<f64>, Option<i64>, &'static str, u8);
}

impl EmptyTupleHelper for (&'static str, u8, Option<i64>, Option<i64>, Option<f64>, Option<i64>) {
    fn map_empty(self) -> (Option<i64>, Option<i64>, Option<f64>, Option<i64>, &'static str, u8) {
        (None, None, None, None, self.0, self.1)
    }
}
