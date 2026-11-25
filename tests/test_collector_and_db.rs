use mobi_market::cli::CliClient;
use mobi_market::collector::Collector;
use mobi_market::store::Database;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_collector_skips_when_already_collecting() {
    let dir = tempdir().unwrap();
    let db = Database::new(&dir.path().join("test_collector.db")).unwrap();
    let cli = Arc::new(CliClient::new());
    let collector = Collector::new(cli, db, 300);

    // 이미 수집 중인 상태 시뮬레이션
    collector.is_collecting.store(true, Ordering::Release);
    let res = collector.run_once().await;
    assert!(res.is_ok());
    // 중복 실행을 건너뛰었으므로 기존 플래그가 유지됨
    assert!(collector.is_collecting.load(Ordering::Acquire));
}

#[tokio::test]
async fn test_database_batch_snapshots_merge_zz_variants() {
    let dir = tempdir().unwrap();
    let db = Database::new(&dir.path().join("test_batch.db")).unwrap();

    db.insert_snapshot("잔영의 숏소드", None, None, None, 1000, true)
        .await
        .unwrap();
    db.insert_snapshot("잔영의 숏소드ZZ", None, None, None, 1200, true)
        .await
        .unwrap();

    let grouped = db.get_all_snapshots_grouped(50).await.unwrap();
    let snaps = grouped.get("잔영의 숏소드").expect("정규화 키 존재 필요");
    assert_eq!(snaps.len(), 2);
    assert_eq!(snaps[0].min_unit_price, 1000);
    assert_eq!(snaps[1].min_unit_price, 1200);

    let history = db.get_snapshot_history("잔영의 숏소드ZZ", 10).await.unwrap();
    assert_eq!(history.len(), 2);
}

#[tokio::test]
async fn test_database_cleans_stale_default_seeds_preserves_user_items() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test_stale_seeds.db");

    // 1. DB 초기화 후 과거 버전 '기본' 유령 품목과 '사용자 추가' 품목을 직접 삽입 시뮬레이션
    {
        let _db = Database::new(&db_path).unwrap();
    }
    let raw_conn = rusqlite::Connection::open(&db_path).unwrap();
    raw_conn
        .execute(
            "INSERT INTO monitored_items (item_name, group_name) VALUES ('단단한 무쇠괴', '기본')",
            [],
        )
        .unwrap();
    raw_conn
        .execute(
            "INSERT INTO monitored_items (item_name, group_name) VALUES ('사용자커스텀품목', '사용자 추가')",
            [],
        )
        .unwrap();
    drop(raw_conn);

    // 2. Database::new 재기동 시 폐기된 '기본' 시드는 삭제되고 '사용자 추가'는 보존되어야 함
    let db = Database::new(&db_path).unwrap();
    let items = db.get_monitored_items().await.unwrap();
    assert!(!items.contains(&"단단한 무쇠괴".to_string()));
    assert!(items.contains(&"사용자커스텀품목".to_string()));
}
