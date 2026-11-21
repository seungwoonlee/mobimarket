#![windows_subsystem = "windows"]

mod calculator;
mod cli;
mod collector;
mod config;
mod gui;
mod store;

use cli::CliClient;
use collector::Collector;
use config::{get_db_path, APP_VERSION, DEFAULT_COLLECT_INTERVAL_SECS};
use gui::MobiMarketApp;
use std::sync::Arc;
use store::Database;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

fn main() -> Result<(), eframe::Error> {
    // 로깅 초기화
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber).ok();

    info!("=== MobiMarket (마켓 시세 추적기 & 제작비용 계산기) 네이티브 GUI 시작 ===");

    // 1. 전역 Tokio 비동기 멀티스레드 런타임 생성
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Tokio 런타임 생성 실패");

    // Tokio 런타임 진입 가드 (block_in_place 및 spawn 지원)
    let _guard = rt.enter();

    // 2. SQLite 데이터베이스 초기화
    let db_path = get_db_path();
    info!("데이터베이스 경로: {:?}", db_path);
    let db = Database::new(&db_path).expect("데이터베이스 초기화 실패");

    // 3. CLI 클라이언트 및 수집기 초기화 (기본 주기: 300초 = 5분)
    let cli = Arc::new(CliClient::new());
    let collector = Collector::new(cli.clone(), db.clone(), DEFAULT_COLLECT_INTERVAL_SECS);

    // 4. 백그라운드 수집 루프 시작 (1초 카운트다운 반응형)
    collector.clone().start_background_loop();

    // 5. 기동 즉시 초기 시세 및 레시피 1회 수집 시도 (비동기 spawn)
    let init_collector = collector.clone();
    rt.spawn(async move {
        info!("초기 마켓 최저가 시세 및 레시피 수집 시작...");
        if let Err(e) = init_collector.run_once().await {
            info!("초기 시세 수집 건너뜀 (게임 미실행 상태): {}", e);
        }
    });

    // 6. eframe 네이티브 윈도우 창 옵션 설정 (1280x720 기본 해상도, 불필요한 설정 파일 저장 방지)
    let window_title = format!("MobiMarket {} - 마비노기 모바일 마켓 시세 & 제작 계산기", APP_VERSION);
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_min_inner_size([1000.0, 580.0])
            .with_title(window_title),
        persist_window: false, // 다른 PC 실행 시 불필요한 AppData 창위치 캐시 파일 생성 방지
        ..Default::default()
    };

    // 7. GUI 앱 실행 (메인 스레드 점유)
    let app_cli = cli.clone();
    let app_db = db.clone();
    let app_collector = collector.clone();

    eframe::run_native(
        "MobiMarket",
        native_options,
        Box::new(move |cc| {
            Ok(Box::new(MobiMarketApp::new(
                cc,
                app_cli,
                app_db,
                app_collector,
            )))
        }),
    )
}
