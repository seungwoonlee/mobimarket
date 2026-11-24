use crate::config::{MaterialRecipeDef, MARKET_FEE_RATE, MATERIAL_RECIPES};
use std::collections::HashMap;

/// 가공 재료 데카벌이 분석 결과 행 (2-Track 원가 및 노력 비교 모델)
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct MaterialProfitRow {
    pub name: String,
    pub category: String,
    pub market_price: Option<i64>,
    pub produced_per_craft: i64,
    pub revenue_net: Option<i64>, // 실수령액 (판매가 * 산출량 * 90%)
    pub duration_secs: u32,
    pub secondary_duration_secs: Option<u32>,

    // 루트 A: 1차 원재료 거래소 직구매 제작 (최소 노력/시간)
    pub primary_ing_name: &'static str,
    pub primary_ing_qty: i64,
    pub primary_cost: Option<i64>,
    pub profit_primary: Option<i64>,
    pub hourly_primary: Option<i64>,

    // 루트 B: 2차 원재료 자체 가공 제작 (노력/시간 추가 투자)
    pub secondary_ing_name: Option<&'static str>,
    pub secondary_cost: Option<i64>,
    pub profit_secondary: Option<i64>,
    pub hourly_secondary: Option<i64>,

    // 종합 추천 전략 및 배지
    pub strategy_label: &'static str,
    pub strategy_code: u8, // 3: 🔥 꿀벌이(자체가공), 2: ⚡ 간편제작(직구매흑자), 1: ⚠️ 직구매적자/전구간적자, 0: 시세대기
    pub is_primary_loss: bool,
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

        // 자체 가공 순이익 내림차순 정렬 (시세 없는 항목은 맨 뒤로)
        results.sort_by(|a, b| {
            let profit_b = b.profit_secondary.or(b.profit_primary).unwrap_or(i64::MIN);
            let profit_a = a.profit_secondary.or(a.profit_primary).unwrap_or(i64::MIN);
            profit_b.cmp(&profit_a)
        });

        results
    }

    /// 단일 품목 분석 (2-Track 모델)
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

        // 1. 완제품 거래소 판매 최저가 및 수수료 10% 제외 실수령액
        let market_price = prices.get(def.name).cloned();
        let revenue_net = market_price.map(|p| {
            (p as f64 * def.produced_per_craft as f64 * (1.0 - MARKET_FEE_RATE)).round() as i64
        });

        // 2. 루트 A: 1차 원재료 직구매 원가 및 순익
        let (primary_name, primary_qty) = def.primary_ingredient;
        let primary_unit_price = prices.get(primary_name).cloned();
        let primary_cost = primary_unit_price.map(|p| p * primary_qty);

        let profit_primary = match (revenue_net, primary_cost) {
            (Some(rev), Some(cost)) => Some(rev - cost),
            _ => None,
        };

        let hourly_primary = profit_primary.map(|net| {
            let per_hour = 3600.0 / duration_secs as f64;
            (per_hour * net as f64).round() as i64
        });

        let is_primary_loss = profit_primary.is_some_and(|p| p < 0);

        // 3. 루트 B: 2차 원재료 자체 가공 원가, 총 소요시간 및 순익
        let (secondary_ing_name, secondary_cost, profit_secondary, hourly_secondary, secondary_duration_secs) =
            if let Some((sec_name, sec_qty, sec_prod)) = def.secondary_ingredient {
                let sec_unit_price = prices.get(sec_name).cloned();
                let sec_cost = sec_unit_price.map(|p| {
                    let unit_cost = (p * sec_qty) as f64 / sec_prod.max(1) as f64;
                    (unit_cost * primary_qty as f64).round() as i64
                });
                let prof_sec = match (revenue_net, sec_cost) {
                    (Some(rev), Some(cost)) => Some(rev - cost),
                    _ => None,
                };
                // 하위 레시피 소요시간이 등록되어 있으면 필요 횟수(primary_qty / sec_prod)만큼 합산, 미등록 시 2배 fallback
                let sec_dur = if let Some(sub_def) = MATERIAL_RECIPES.iter().find(|r| r.name == primary_name) {
                    let sub_unit_dur = custom_durations
                        .get(sub_def.name)
                        .cloned()
                        .unwrap_or(sub_def.default_duration_secs)
                        .max(1);
                    let sub_crafts = (primary_qty as f64) / (sec_prod.max(1) as f64);
                    (duration_secs as f64 + (sub_unit_dur as f64) * sub_crafts)
                        .round()
                        .max(1.0) as u32
                } else {
                    duration_secs.saturating_mul(2)
                };
                let hour_sec = prof_sec.map(|net| {
                    let per_hour = 3600.0 / sec_dur as f64;
                    (per_hour * net as f64).round() as i64
                });
                (Some(sec_name), sec_cost, prof_sec, hour_sec, Some(sec_dur))
            } else {
                (None, None, None, None, None)
            };

        // 4. 종합 추천 전략 라벨 결정
        let (strategy_label, strategy_code) = match (profit_primary, profit_secondary) {
            (Some(p1), Some(p2)) => {
                if p1 < 0 && p2 > 0 {
                    ("⚠️ 직구매 적자 (자체 가공 필수)", 1)
                } else if p1 < 0 && p2 <= 0 {
                    ("⚠️ 전 구간 적자 주의", 1)
                } else if p2 >= 70 {
                    ("🔥 꿀벌이 (자체 가공 추천)", 3)
                } else if p1 > 0 {
                    ("⚡ 간편 제작 (직구매도 흑자)", 2)
                } else {
                    ("⚠️ 적자 주의", 1)
                }
            }
            (Some(p1), None) => {
                if p1 > 0 {
                    ("⚡ 간편 제작 추천", 2)
                } else {
                    ("⚠️ 직구매 적자", 1)
                }
            }
            _ => ("⚪ 시세 대기 중", 0),
        };

        MaterialProfitRow {
            name: def.name.to_string(),
            category: def.category.to_string(),
            market_price,
            produced_per_craft: def.produced_per_craft,
            revenue_net,
            duration_secs,
            secondary_duration_secs,
            primary_ing_name: primary_name,
            primary_ing_qty: primary_qty,
            primary_cost,
            profit_primary,
            hourly_primary,
            secondary_ing_name,
            secondary_cost,
            profit_secondary,
            hourly_secondary,
            strategy_label,
            strategy_code,
            is_primary_loss,
        }
    }
}
