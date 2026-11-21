use mobi_market::calculator::MaterialProfitCalculator;
use std::collections::HashMap;

#[test]
fn test_material_profit_calculation_platinum_ingot() {
    let mut prices = HashMap::new();
    prices.insert("백금강괴".to_string(), 50_000);
    prices.insert("백금괴".to_string(), 10_000);
    prices.insert("철괴".to_string(), 2_000);
    prices.insert("석탄".to_string(), 1_000);

    let custom_durations = HashMap::new();
    let results = MaterialProfitCalculator::calculate(&prices, &custom_durations);

    let row = results.iter().find(|r| r.name == "백금강괴").expect("백금강괴 결과 필요");
    assert_eq!(row.market_price, Some(50_000));
    assert_eq!(row.material_cost, Some(33_000)); // 10000*2 + 2000*5 + 1000*3

    // 실수령: 50,000 * 0.9 = 45,000
    // 순이익: 45,000 - 33,000 = 12,000
    assert_eq!(row.net_profit_per_unit, Some(12_000));
    assert_eq!(row.duration_secs, 6);

    // 시간당 제작 횟수 = 3600 / 6 = 600
    // 시간당 데카 = 600 * 12,000 = 7,200,000
    assert_eq!(row.hourly_profit, Some(7_200_000));
    assert_eq!(row.status_code, 4); // 꿀벌이
}

#[test]
fn test_material_profit_calculation_produced_per_craft() {
    let mut prices = HashMap::new();
    // 질긴 가죽끈: 1회 2개 생산
    prices.insert("질긴 가죽끈".to_string(), 5_000);
    prices.insert("고급 가죽".to_string(), 3_000);
    prices.insert("굵은 실뭉치".to_string(), 500);

    let custom_durations = HashMap::new();
    let results = MaterialProfitCalculator::calculate(&prices, &custom_durations);

    let row = results.iter().find(|r| r.name == "질긴 가죽끈").expect("질긴 가죽끈 결과 필요");
    assert_eq!(row.produced_per_craft, 2);
    assert_eq!(row.market_price, Some(5_000));
    assert_eq!(row.material_cost, Some(7_500)); // 3000*2 + 500*3 = 7,500

    // 1회 매출(2개 * 5000 * 0.9) = 9,000
    // 1회 순마진 = 9,000 - 7,500 = 1,500
    // 개당 순마진 = 1,500 / 2 = 750
    assert_eq!(row.net_profit_per_unit, Some(750));
    assert_eq!(row.duration_secs, 4);

    // 시간당 횟수 = 3600 / 4 = 900
    // 시간당 데카 = 900 * 1500 = 1,350,000
    assert_eq!(row.hourly_profit, Some(1_350_000));
}

#[test]
fn test_material_profit_missing_prices() {
    let prices = HashMap::new(); // 시세 비어있음
    let custom_durations = HashMap::new();
    let results = MaterialProfitCalculator::calculate(&prices, &custom_durations);

    for row in results {
        assert_eq!(row.market_price, None);
        assert_eq!(row.material_cost, None);
        assert_eq!(row.net_profit_per_unit, None);
        assert_eq!(row.hourly_profit, None);
        assert_eq!(row.status_code, 0); // 시세 대기 중
    }
}
