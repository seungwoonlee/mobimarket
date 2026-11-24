use mobi_market::calculator::MaterialProfitCalculator;
use std::collections::HashMap;

#[test]
fn test_material_profit_calculation_special_silk() {
    let mut prices = HashMap::new();
    // 특급 실크: 최상급 실크 4개 투입 -> 2개 산출, 상급 실크 4개로 최상급 실크 2개 산출
    prices.insert("특급 실크".to_string(), 120);
    prices.insert("최상급 실크".to_string(), 45);
    prices.insert("상급 실크".to_string(), 10);

    let custom_durations = HashMap::new();
    let results = MaterialProfitCalculator::calculate(&prices, &custom_durations);

    let row = results.iter().find(|r| r.name == "특급 실크").expect("특급 실크 결과 필요");
    assert_eq!(row.produced_per_craft, 2);
    assert_eq!(row.market_price, Some(120));

    // 실수령: 120 * 2 * 0.9 = 216
    assert_eq!(row.revenue_net, Some(216));

    // 루트 A: 1차 최상급 실크 4개 직구매 (45 * 4 = 180) -> 순익 216 - 180 = +36
    assert_eq!(row.primary_cost, Some(180));
    assert_eq!(row.profit_primary, Some(36));
    assert!(!row.is_primary_loss);

    // 루트 B: 2차 상급 실크 4개로 최상급 실크 2개 산출 -> 개당 20데카
    // 최상급 실크 4개 투입 시 환산원가: 20 * 4 = 80데카
    // 순익: 216 - 80 = +136데카
    assert_eq!(row.secondary_cost, Some(80));
    assert_eq!(row.profit_secondary, Some(136));
    assert_eq!(row.secondary_duration_secs, Some(420)); // 180 + 120 * (4/2) = 420초
    assert_eq!(row.strategy_code, 3); // 꿀벌이 (자체 가공 추천)
}

#[test]
fn test_material_profit_calculation_platinum_loss_warning() {
    let mut prices = HashMap::new();
    // 백금강괴: 운철괴 5개 투입 -> 3개 산출
    prices.insert("백금강괴".to_string(), 130);
    prices.insert("운철괴".to_string(), 74);
    prices.insert("은합금괴".to_string(), 37);

    let custom_durations = HashMap::new();
    let results = MaterialProfitCalculator::calculate(&prices, &custom_durations);

    let row = results.iter().find(|r| r.name == "백금강괴").expect("백금강괴 결과 필요");
    assert_eq!(row.produced_per_craft, 3);

    // 실수령: 130 * 3 * 0.9 = 351
    assert_eq!(row.revenue_net, Some(351));

    // 루트 A: 운철괴 5개 직구매 (74 * 5 = 370) -> 순익 351 - 370 = -19 (적자!)
    assert_eq!(row.primary_cost, Some(370));
    assert_eq!(row.profit_primary, Some(-19));
    assert!(row.is_primary_loss);

    // 루트 B: 은합금괴 5개(37*5=185)로 운철괴 3개 산출 -> 운철괴 개당 원가 61.67데카
    // 운철괴 5개 투입 시 환산원가: 61.67 * 5 = 308데카
    // 순익: 351 - 308 = +43 (흑자 전환!)
    assert_eq!(row.secondary_cost, Some(308));
    assert_eq!(row.profit_secondary, Some(43));
    assert_eq!(row.strategy_code, 1); // ⚠️ 직구매 적자 (자체 가공 필수)
}

#[test]
fn test_material_profit_missing_prices() {
    let prices = HashMap::new(); // 시세 비어있음
    let custom_durations = HashMap::new();
    let results = MaterialProfitCalculator::calculate(&prices, &custom_durations);

    for row in results {
        assert_eq!(row.market_price, None);
        assert_eq!(row.revenue_net, None);
        assert_eq!(row.profit_primary, None);
        assert_eq!(row.profit_secondary, None);
        assert_eq!(row.strategy_code, 0); // 시세 대기 중
    }
}
