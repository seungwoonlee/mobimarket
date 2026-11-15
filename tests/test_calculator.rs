use mobi_market::calculator::CraftCalculator;
use mobi_market::cli::models::OwnedItem;
use mobi_market::store::Database;
use tempfile::tempdir;

#[tokio::test]
async fn test_craft_calculator_savings() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test_market.db");
    let db = Database::new(&db_path).unwrap();

    // 1. 레시피 등록 (성공률 90.0%, 대성공률 12.0%)
    let ingredients_json = serde_json::json!([
        { "DisplayName": "특급 가죽", "Required": 4, "Owned": 0 },
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

    // 4. 가방 + 내창고 + 공용창고 아이템 목업 (가방 1개, 내창고 1개 -> 총 2개 보유)
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
    ];

    let calc = CraftCalculator::new(db);

    // Case 1: 성공률 감안 OFF, 대성공 감안 OFF (기본 1회 원가)
    let res_base = calc.calculate("해연의 가죽 갑옷 장갑ZZ", 1, &owned, false, false).await.unwrap();
    assert_eq!(res_base.product_total_market_price, Some(4458));
    assert_eq!(res_base.total_material_cost, 1488);
    // 특급 가죽 필요 4개 중 2개 보유 -> 2개 부족 (256데카) + 포식의 마력석 8개 부족 (976데카) = 1,232데카
    assert_eq!(res_base.user_inventory_gap_cost, 1232);
    assert_eq!(res_base.saving_amount, Some(4458 - 1488));
    assert!(res_base.saving_amount.unwrap() > 0);
    // 4458 vs 1488: 절감율 (4458-1488)/4458 = 66.6% >= 10% -> 강력 추천
    assert!(res_base.recommendation.contains("강력 추천"));

    // Case 2: 성공률 감안 ON (90%), 대성공 감안 OFF
    // 1 / 0.90 = 1.1111회 시도 필요
    // 실구매 개수 정수 올림(ceil): 가죽 ceil(4 * 1.1111) = 5개 (640데카), 마력석 ceil(8 * 1.1111) = 9개 (1098데카)
    // 기대 제작비 = 640 + 1098 = 1,738데카
    let res_succ = calc.calculate("해연의 가죽 갑옷 장갑ZZ", 1, &owned, true, false).await.unwrap();
    assert_eq!(res_succ.total_material_cost, 1738);
    // 부족 수량: 가죽 5개 중 2개 보유 -> 3개 부족 (384데카) + 마력석 9개 부족 (1098데카) = 1,482데카
    assert_eq!(res_succ.user_inventory_gap_cost, 1482);
    assert_eq!(res_succ.ingredients[0].expected_required, 5);
    assert_eq!(res_succ.ingredients[0].shortage_count, 3);
    assert_eq!(res_succ.ingredients[1].expected_required, 9);
    assert_eq!(res_succ.ingredients[1].shortage_count, 9);
    assert!(res_succ.expected_attempts > 1.11 && res_succ.expected_attempts < 1.12);
}






