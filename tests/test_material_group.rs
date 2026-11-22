use mobi_market::config::{GROUP_MATERIALS, DEFAULT_MONITORED_NAMES};
use mobi_market::gui::ItemGroup;
use mobi_market::store::Database;
use tempfile::tempdir;

#[tokio::test]
async fn test_material_group_constants_and_label() {
    // 1. ItemGroup::Materials 라벨 검증
    assert_eq!(ItemGroup::Materials.label(), "재료");

    // 2. 핵심 레시피 재료들이 GROUP_MATERIALS에 모두 포함되어 있는지 검증
    let required_materials = [
        "공명의 영혼석",
        "파동의 영혼석",
        "망령의 영혼석",
        "원념의 영혼석",
        "삼림의 영혼석",
        "야생의 영혼석",
        "허상의 마력석",
        "포식의 마력석",
        "심해의 마력석",
        "백금강괴",
        "운철괴",
        "은합금괴",
        "특급 목재",
        "최상급 목재+",
        "특급 가죽",
        "최상급 가죽+",
        "특급 옷감",
        "최상급 옷감+",
        "특급 실크",
        "최상급 실크",
        "세공된 페리도트ZZ",
    ];

    for &mat in &required_materials {
        assert!(
            GROUP_MATERIALS.contains(&mat),
            "GROUP_MATERIALS에 필수 재료 '{}'가 누락되었습니다.",
            mat
        );
        assert!(
            DEFAULT_MONITORED_NAMES.contains(&mat),
            "DEFAULT_MONITORED_NAMES에 필수 재료 '{}'가 누락되었습니다.",
            mat
        );
    }
}

#[tokio::test]
async fn test_database_material_seed_and_ignore() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test_material.db");

    // DB 초기화 (시드 등록 및 마이그레이션 v2 적용)
    let db = Database::new(&db_path).expect("DB 생성 실패");

    let items = db.get_monitored_items().await.unwrap();

    // 1. 기본 모니터링 품목에 재료들이 정상 시드 등록되었는지 검증
    assert!(items.contains(&"특급 실크".to_string()));
    assert!(items.contains(&"백금강괴".to_string()));
    assert!(items.contains(&"삼림의 영혼석".to_string()));
    assert!(items.contains(&"세공된 페리도트ZZ".to_string()));

    // 2. add_monitored_item_ignore 무결성 검증 (중복 추가 시 오류 없이 기존 데이터 보존)
    let res = db.add_monitored_item_ignore("특급 실크", "재료").await;
    assert!(res.is_ok());

    let res_new = db.add_monitored_item_ignore("신규테스트원재료", "재료").await;
    assert!(res_new.is_ok());

    let updated_items = db.get_monitored_items().await.unwrap();
    assert!(updated_items.contains(&"신규테스트원재료".to_string()));
}
