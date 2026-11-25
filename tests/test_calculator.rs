use mobi_market::calculator::CraftCalculator;
use mobi_market::cli::models::OwnedItem;
use mobi_market::config::{format_number, format_signed_number, is_zero_cost_material};
use mobi_market::store::Database;
use tempfile::tempdir;

#[tokio::test]
async fn test_craft_calculator_savings() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test_market.db");
    let db = Database::new(&db_path).unwrap();

    // 1. 레시피 등록 (성공률 90.0%, 대성공률 12.0%)
    let ingredients_json = serde_json::json!([
        { "DisplayName": "특급 가죽", "Required": 4, "Owned": 99 },
        { "DisplayName": "포식의 마력석", "Required": 8, "Owned": 0 }
    ])
    .to_string();

    db.upsert_recipe("해연의 가죽 갑옷 장갑ZZ", 1, 90.0, 12.0, &ingredients_json)
        .await
        .unwrap();

    // 2. 완제품 시세 (4,458 데카)
    db.insert_snapshot("해연의 가죽 갑옷 장갑ZZ", None, None, None, 4458, true)
        .await
        .unwrap();

    // 3. 재료 시세 (특급 가죽 128데카 x 4 = 512, 포식의 마력석 122데카 x 8 = 976 -> 기본 총 1,488데카)
    db.insert_snapshot("특급 가죽", None, None, None, 128, true)
        .await
        .unwrap();
    db.insert_snapshot("포식의 마력석", None, None, None, 122, true)
        .await
        .unwrap();

    // 4. 가방 + 내창고 + 잠금 아이템 목업 (가방 1개, 내창고 1개, 잠금 5개 -> 잠금 제외 총 2개 보유)
    let owned = vec![
        OwnedItem {
            location: "inventory".to_string(),
            display_name: "특급 가죽".to_string(),
            count: 1,
            is_locked: false,
        },
        OwnedItem {
            location: "character_storage".to_string(),
            display_name: "특급 가죽".to_string(),
            count: 1,
            is_locked: false,
        },
        OwnedItem {
            location: "account_storage".to_string(),
            display_name: "특급 가죽".to_string(),
            count: 5,
            is_locked: true,
        },
    ];

    let calc = CraftCalculator::new(db);

    // Case 1: 성공률 감안 OFF, 대성공 감안 OFF (실시간 인벤토리 연동: ing.owned=99 무시, 잠금 5개 제외)
    let res_base = calc
        .calculate("해연의 가죽 갑옷 장갑ZZ", 1, &owned, true, false, false)
        .await
        .unwrap();
    assert!(!res_base.has_missing_prices);
    assert_eq!(res_base.product_total_market_price, Some(4458));
    assert_eq!(res_base.total_material_cost, 1488);
    // 특급 가죽 필요 4개 중 2개 보유 -> 2개 부족 (256데카) + 포식의 마력석 8개 부족 (976데카) = 1,232데카
    assert_eq!(res_base.user_inventory_gap_cost, 1232);
    assert_eq!(res_base.saving_amount, Some(4458 - 1488));
    // 10% 거래소 수수료 공제: round(4458 * 0.90 - 1488) = 4012 - 1488 = 2524
    assert_eq!(res_base.net_profit, Some(2524));
    assert!(res_base.recommendation.contains("강력 추천"));

    // Case 2: 성공률 감안 ON (90%), 대성공 감안 OFF
    let res_succ = calc
        .calculate("해연의 가죽 갑옷 장갑ZZ", 1, &owned, true, true, false)
        .await
        .unwrap();
    assert_eq!(res_succ.total_material_cost, 1738);
    assert_eq!(res_succ.user_inventory_gap_cost, 1482);
    assert_eq!(res_succ.ingredients[0].expected_required, 5);
    assert_eq!(res_succ.ingredients[0].shortage_count, 3);
    assert_eq!(res_succ.ingredients[1].expected_required, 9);
    assert_eq!(res_succ.ingredients[1].shortage_count, 9);
    assert!(res_succ.expected_attempts > 1.11 && res_succ.expected_attempts < 1.12);

    // Case 3: 실시간 인벤토리가 비어있어도 연동 상태면 과거 ing.owned(99)로 fallback하지 않음
    let res_empty_live = calc
        .calculate("해연의 가죽 갑옷 장갑ZZ", 1, &[], true, false, false)
        .await
        .unwrap();
    assert_eq!(res_empty_live.ingredients[0].owned_count, 0);
    assert_eq!(res_empty_live.user_inventory_gap_cost, 1488);

    // Case 4: 잘못된 craft_count <= 0 거부
    assert!(calc
        .calculate("해연의 가죽 갑옷 장갑ZZ", 0, &[], true, false, false)
        .await
        .is_err());
}

#[tokio::test]
async fn test_craft_calculator_missing_material_price_blocks_false_recommendation() {
    let dir = tempdir().unwrap();
    let db = Database::new(&dir.path().join("test_missing.db")).unwrap();

    let ingredients_json = serde_json::json!([
        { "DisplayName": "시세없는고가재료", "Required": 2, "Owned": 0 }
    ])
    .to_string();
    db.upsert_recipe("테스트 장비ZZ", 1, 100.0, 0.0, &ingredients_json)
        .await
        .unwrap();
    db.insert_snapshot("테스트 장비ZZ", None, None, None, 5000, true)
        .await
        .unwrap();

    let calc = CraftCalculator::new(db);
    let res = calc
        .calculate("테스트 장비ZZ", 1, &[], true, false, false)
        .await
        .unwrap();
    assert!(res.has_missing_prices);
    assert_eq!(res.missing_price_items, vec!["시세없는고가재료".to_string()]);
    assert_eq!(res.saving_amount, None);
    assert_eq!(res.net_profit, None);
    assert!(res.recommendation.contains("미수집"));
}

#[tokio::test]
async fn test_craft_calculator_produced_per_craft_and_number_formatting() {
    let dir = tempdir().unwrap();
    let db = Database::new(&dir.path().join("test_multi_prod.db")).unwrap();

    let ingredients_json = serde_json::json!([
        { "DisplayName": "상급 실크", "Required": 4, "Owned": 0 }
    ])
    .to_string();
    db.upsert_recipe("최상급 실크", 2, 100.0, 10.0, &ingredients_json)
        .await
        .unwrap();
    db.insert_snapshot("최상급 실크", None, None, None, 100, true)
        .await
        .unwrap();
    db.insert_snapshot("상급 실크", None, None, None, 10, true)
        .await
        .unwrap();

    let calc = CraftCalculator::new(db);
    let res = calc
        .calculate("최상급 실크", 1, &[], true, true, true)
        .await
        .unwrap();
    // produced_per_craft=2, crit=10% -> expected_yield=2.1 -> attempts=2/2.1=0.95238...
    assert_eq!(res.produced_quantity, 2);
    assert!(res.expected_attempts < 1.0);
    assert_eq!(res.net_profit, Some((200.0_f64 * 0.90).round() as i64 - 40));

    // 숫자 포맷터 음수/양수/i64::MIN 안전성 검증
    assert_eq!(format_number(-100), "-100");
    assert_eq!(format_number(-1234567), "-1,234,567");
    assert_eq!(format_signed_number(136), "+136");
    assert_eq!(format_signed_number(-19), "-19");
    assert_eq!(format_signed_number(0), "0");
    assert_eq!(format_number(i64::MIN), "-9,223,372,036,854,775,808");
}

#[tokio::test]
async fn test_zero_cost_peridot_treated_as_zero_deca_without_warning() {
    assert!(is_zero_cost_material("세공된 페리도트"));
    assert!(is_zero_cost_material("세공된 페리도트ZZ"));
    assert!(is_zero_cost_material(" 세공된 페리도트ZZ "));
    assert!(!is_zero_cost_material("특급 가죽"));

    let dir = tempdir().unwrap();
    let db = Database::new(&dir.path().join("test_peridot.db")).unwrap();

    // 세공된 페리도트ZZ 시세가 DB에 전혀 없어도, 다른 재료 및 완제품 시세만 있으면 정상 추천되어야 함
    let ingredients_json = serde_json::json!([
        { "DisplayName": "세공된 페리도트ZZ", "Required": 10, "Owned": 0 },
        { "DisplayName": "포식의 마력석", "Required": 5, "Owned": 0 }
    ])
    .to_string();
    db.upsert_recipe("잔영의 페리도트 링ZZ", 1, 100.0, 0.0, &ingredients_json)
        .await
        .unwrap();
    db.insert_snapshot("잔영의 페리도트 링ZZ", None, None, None, 3000, true)
        .await
        .unwrap();
    db.insert_snapshot("포식의 마력석", None, None, None, 100, true)
        .await
        .unwrap();

    let calc = CraftCalculator::new(db.clone());
    let res = calc
        .calculate("잔영의 페리도트 링ZZ", 1, &[], true, true, false)
        .await
        .unwrap();

    assert!(!res.has_missing_prices);
    assert!(res.missing_price_items.is_empty());
    assert_eq!(res.ingredients[0].unit_min_price, Some(0));
    assert_eq!(res.ingredients[0].total_cost, Some(0));
    assert_eq!(res.ingredients[0].gap_cost, Some(0));
    assert_eq!(res.total_material_cost, 500);
    assert_eq!(res.saving_amount, Some(2500));
    assert!(res.recommendation.contains("직접 제작 강력 추천"));

    // DB에 임의 고가 스냅샷(9999 데카)이 들어가더라도 원가 계산은 항상 0데카로 고정
    db.insert_snapshot("세공된 페리도트ZZ", None, None, None, 9999, true)
        .await
        .unwrap();
    let res_with_snapshot = calc
        .calculate("잔영의 페리도트 링ZZ", 1, &[], true, true, false)
        .await
        .unwrap();
    assert_eq!(res_with_snapshot.ingredients[0].unit_min_price, Some(0));
    assert_eq!(res_with_snapshot.total_material_cost, 500);
}

#[tokio::test]
async fn test_opportunity_cost_sell_materials_vs_craft_and_all_recipes() {
    let dir = tempdir().unwrap();
    let db = Database::new(&dir.path().join("test_opp_cost.db")).unwrap();

    // Case 1: 완제품 1,000데카, 재료 시세 합 1,050데카(전량 구매 시 50 손해)이지만
    // 재료를 100% 보유 중이면 매각 시 10% 수수료 공제 후 945데카만 회수 가능 -> 직접 제작이 55데카 유리!
    let ing1 = serde_json::json!([
        { "DisplayName": "특급 가죽", "Required": 10, "Owned": 0 }
    ])
    .to_string();
    db.upsert_recipe("잔영의 갑옷ZZ", 1, 100.0, 0.0, &ing1).await.unwrap();
    db.insert_snapshot("잔영의 갑옷ZZ", None, None, None, 1000, true).await.unwrap();
    db.insert_snapshot("특급 가죽", None, None, None, 105, true).await.unwrap();

    // Case 2: 완제품 1,000데카, 재료 시세 합 1,200데카 -> 보유 재료 매각 시 수수료 10% 후 1,080데카!
    // 보유 재료만 팔아도 완제품(1,000) 구매 가능하고 +80데카 잉여 -> 매각 후 완제품 구매 강력 추천!
    let ing2 = serde_json::json!([
        { "DisplayName": "특급 실크", "Required": 10, "Owned": 0 }
    ])
    .to_string();
    db.upsert_recipe("잔영의 전투복ZZ", 1, 100.0, 0.0, &ing2).await.unwrap();
    db.insert_snapshot("잔영의 전투복ZZ", None, None, None, 1000, true).await.unwrap();
    db.insert_snapshot("특급 실크", None, None, None, 120, true).await.unwrap();

    // Case 3: 일부 보유 + 일부 부족 (완제품 2,000 / 재료 A 10개*180 보유 / 재료 B 5개*100 부족)
    // 직접 제작 시 추가 지출 = 500, 매각 후 구매 시 순부담 = 2000 - round(1800*0.9)=1620 -> 380!
    // 매각 후 구매가 120데카 더 저렴!
    let ing3 = serde_json::json!([
        { "DisplayName": "백금강괴", "Required": 10, "Owned": 0 },
        { "DisplayName": "운철괴", "Required": 5, "Owned": 0 }
    ])
    .to_string();
    db.upsert_recipe("잔영의 숏소드ZZ", 1, 100.0, 0.0, &ing3).await.unwrap();
    db.insert_snapshot("잔영의 숏소드ZZ", None, None, None, 2000, true).await.unwrap();
    db.insert_snapshot("백금강괴", None, None, None, 180, true).await.unwrap();
    db.insert_snapshot("운철괴", None, None, None, 100, true).await.unwrap();

    let owned = vec![
        OwnedItem {
            location: "inventory".to_string(),
            display_name: "특급 가죽".to_string(),
            count: 10,
            is_locked: false,
        },
        OwnedItem {
            location: "character_storage".to_string(),
            display_name: "특급 실크".to_string(),
            count: 20, // 투입분 10개보다 많음 -> used_owned=10, total_owned=20
            is_locked: false,
        },
        OwnedItem {
            location: "account_storage".to_string(),
            display_name: "백금강괴".to_string(),
            count: 10,
            is_locked: false,
        },
    ];

    let calc = CraftCalculator::new(db.clone());

    // 검증 1: 수수료 역전 케이스
    let r1 = calc.calculate("잔영의 갑옷ZZ", 1, &owned, true, true, false).await.unwrap();
    assert_eq!(r1.owned_materials_sell_gross, 1050);
    assert_eq!(r1.owned_materials_sell_net, 945);
    assert_eq!(r1.buy_after_selling_net_cost, Some(55));
    assert_eq!(r1.user_inventory_gap_cost, 0);
    assert_eq!(r1.craft_advantage_over_sell_buy, Some(55));
    assert!(!r1.can_buy_with_used_owned);
    assert!(r1.recommendation.contains("직접 제작 추천"));

    // 검증 2: 보유 재료 매각만으로 완제품 구매 + 잉여금
    let r2 = calc.calculate("잔영의 전투복ZZ", 1, &owned, true, true, false).await.unwrap();
    assert_eq!(r2.owned_materials_sell_net, 1080);
    assert_eq!(r2.all_owned_materials_sell_net, 2160);
    assert_eq!(r2.buy_after_selling_net_cost, Some(-80));
    assert!(r2.can_buy_with_used_owned);
    assert!(r2.can_buy_with_all_owned);
    assert_eq!(r2.owned_sell_surplus, Some(80));
    assert_eq!(r2.craft_advantage_over_sell_buy, Some(-80));
    assert!(r2.recommendation.contains("보유 재료만 팔아도 구매 가능"));

    // 검증 3: 일부 보유 + 일부 부족 시 매각 후 구매 유리
    let r3 = calc.calculate("잔영의 숏소드ZZ", 1, &owned, true, true, false).await.unwrap();
    assert_eq!(r3.owned_materials_sell_net, 1620);
    assert_eq!(r3.user_inventory_gap_cost, 500);
    assert_eq!(r3.buy_after_selling_net_cost, Some(380));
    assert_eq!(r3.craft_advantage_over_sell_buy, Some(-120));
    assert!(r3.recommendation.contains("매각 후 마켓 완제품 구매"));

    // 검증 4: 전품목 일괄 비교
    let recipes = db.get_recipes(None).await.unwrap();
    let all_rows = calc.evaluate_all_recipes(&recipes, 1, &owned, true, true, false).await.unwrap();
    assert_eq!(all_rows.len(), 3);
    let row_armor = all_rows.iter().find(|r| r.display_name == "잔영의 갑옷ZZ").unwrap();
    let row_cloth = all_rows.iter().find(|r| r.display_name == "잔영의 전투복ZZ").unwrap();
    let row_sword = all_rows.iter().find(|r| r.display_name == "잔영의 숏소드ZZ").unwrap();
    assert_eq!(row_armor.status_code, 1);
    assert_eq!(row_cloth.status_code, 3);
    assert_eq!(row_sword.status_code, 2);
}






