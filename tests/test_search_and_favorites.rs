#[test]
fn test_whitespace_normalization_for_search() {
    fn normalize_search_text(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase()
    }

    assert_eq!(normalize_search_text("특급목재"), "특급목재");
    assert_eq!(normalize_search_text("특급 목재"), "특급목재");
    assert_eq!(normalize_search_text(" 특 급   목 재 "), "특급목재");
    assert_eq!(normalize_search_text("백금강괴"), "백금강괴");
    assert_eq!(normalize_search_text("백금 강괴"), "백금강괴");
    assert_eq!(normalize_search_text("Special Silk"), "specialsilk");
    assert_eq!(normalize_search_text("special silk"), "specialsilk");

    // 부분 일치 검증
    let target = normalize_search_text("특급 목재");
    let query1 = normalize_search_text("특급목재");
    let query2 = normalize_search_text("특급");
    let query3 = normalize_search_text("목재");
    let query4 = normalize_search_text("특급 목");

    assert!(target.contains(&query1));
    assert!(target.contains(&query2));
    assert!(target.contains(&query3));
    assert!(target.contains(&query4));
}
