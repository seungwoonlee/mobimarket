use mobi_market::store::Database;
use tempfile::tempdir;

#[tokio::test]
async fn test_favorites_crud_and_deduplication() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test_fav.db");
    let db = Database::new(&db_path).expect("DB 생성 실패");

    // 초기 상태 확인 (0개)
    let favs = db.get_all_favorites().await.expect("조회 실패");
    assert!(favs.is_empty());

    // 1. 찜 추가
    db.add_favorite("마력 깃든 용비늘").await.expect("추가 실패");
    db.add_favorite("듀얼소드ZZ").await.expect("추가 실패");

    let favs = db.get_all_favorites().await.expect("조회 실패");
    assert_eq!(favs.len(), 2);
    assert!(favs.contains("마력 깃든 용비늘"));
    assert!(favs.contains("듀얼소드ZZ"));

    // 2. 중복 찜 시도 (INSERT OR IGNORE 검증)
    db.add_favorite("마력 깃든 용비늘").await.expect("중복 추가 시도 실패");
    let favs = db.get_all_favorites().await.expect("조회 실패");
    assert_eq!(favs.len(), 2);

    // 3. 찜 삭제
    db.remove_favorite("마력 깃든 용비늘").await.expect("삭제 실패");
    let favs = db.get_all_favorites().await.expect("조회 실패");
    assert_eq!(favs.len(), 1);
    assert!(!favs.contains("마력 깃든 용비늘"));
    assert!(favs.contains("듀얼소드ZZ"));
}
