use crate::calculator::{CraftCalculationResult, CraftCalculator, MaterialProfitCalculator};
use crate::cli::models::{MarketRow, OwnedItem};
use crate::cli::CliClient;
use crate::collector::Collector;
use crate::config::{
    get_equipment_slot, is_job_equipment, EquipmentSlot, APP_VERSION, GROUP_BAGS, GROUP_BOXES, GROUP_HIGH_VALUE,
    GROUP_MAGIC_STONES, GROUP_SOUL_STONES,
};
use crate::store::{Database, RecipeRecord, SnapshotRecord};
use eframe::egui::{self, Color32, RichText, Stroke};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MainTab {
    MarketPrices,
    CraftCalculator,
    TimingAnalysis,
    MaterialProfit,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MaterialSortMode {
    SecondaryProfit, // 자체 가공 순익순 (기본)
    PrimaryProfit,   // 직구매 순익순
    HourlyProfit,    // 시간당 시급순
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ItemGroup {
    Favorites,
    All,
    HighValue,
    MagicStones,
    SoulStones,
    Boxes,
    Bags,
    Remnant,
    Abyss,
    MyJob,
}

impl ItemGroup {
    pub fn label(&self) -> &'static str {
        match self {
            ItemGroup::Favorites => "⭐ 찜 목록",
            ItemGroup::All => "전체",
            ItemGroup::HighValue => "고가템",
            ItemGroup::MagicStones => "마력석",
            ItemGroup::SoulStones => "영혼석",
            ItemGroup::Boxes => "상자류",
            ItemGroup::Bags => "컬렉션 백",
            ItemGroup::Remnant => "잔영 장비",
            ItemGroup::Abyss => "해연 장비",
            ItemGroup::MyJob => "내 직업 장비",
        }
    }
}

pub struct MobiMarketApp {
    cli: Arc<CliClient>,
    db: Database,
    collector: Collector,

    // 네비게이션 상태
    active_tab: MainTab,

    // 자동 수집 주기 슬라이더 (분 단위, 기본 10분)
    interval_minutes: u32,

    // 게임 상태 캐시
    cli_connected: bool,
    character_info: String,
    job_name: String,
    owned_items_cache: Vec<OwnedItem>,

    // [시세 탭 상태]
    selected_group: ItemGroup,
    selected_item: String,
    monitored_items: Vec<String>,
    latest_prices: HashMap<String, i64>,
    snapshots_cache: HashMap<String, Vec<SnapshotRecord>>,
    period_days: u32, // 조회 기간 슬라이더 (기본 7일, 0은 전체)

    // 거래소 전체 실시간 검색 기능
    market_search_keyword: String,
    market_search_results: Vec<MarketRow>,
    is_searching_market: bool,
    search_error_msg: Option<String>,

    // [제작 계산기 탭 상태]
    job_only_filter: bool,
    craft_grade_filter: String, // "전체", "해연", "잔영"
    craft_slot_filter: Option<EquipmentSlot>, // None(전체), Weapon(무기), Armor(방어구), Accessory(장신구)
    craft_search_query: String,
    selected_recipe: Option<RecipeRecord>,
    craft_quantity: i32,
    craft_consider_success: bool, // 성공률 감안 기대값 (기본 ON)
    craft_consider_critical: bool, // 대성공 감안 기대값 (기본 OFF)
    cost_calculation: Option<CraftCalculationResult>,

    // 레시피 목록 캐시
    all_recipes: Vec<RecipeRecord>,

    // AI 커넥터 안내 팝업 상태
    show_ai_connector_popup: bool,
    has_shown_ai_connector_popup: bool,
    ai_connector_retry_failed: bool,

    // [찜(즐겨찾기) 캐시]
    favorites: HashSet<String>,

    // [가공 제작재료 데카벌이 계산기 상태]
    material_batch_count: i32,
    custom_material_durations: HashMap<String, u32>,
    material_sort_mode: MaterialSortMode,
    selected_material_name: Option<String>,

    // 마지막 갱신 틱 카운터
    last_refresh_tick: std::time::Instant,
}

impl MobiMarketApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        cli: Arc<CliClient>,
        db: Database,
        collector: Collector,
    ) -> Self {
        setup_custom_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let mut app = Self {
            cli,
            db,
            collector,
            active_tab: MainTab::MarketPrices,
            interval_minutes: 10, // 기본 10분
            cli_connected: false,
            character_info: "미연동".to_string(),
            job_name: "미연동".to_string(),
            owned_items_cache: Vec::new(),
            selected_group: ItemGroup::All,
            selected_item: "마력 깃든 용비늘".to_string(),
            monitored_items: Vec::new(),
            latest_prices: HashMap::new(),
            snapshots_cache: HashMap::new(),
            period_days: 7, // 기본 7일 조회
            market_search_keyword: String::new(),
            market_search_results: Vec::new(),
            is_searching_market: false,
            search_error_msg: None,
            job_only_filter: true,
            craft_grade_filter: "전체".to_string(),
            craft_slot_filter: None,
            craft_search_query: String::new(),
            selected_recipe: None,
            craft_quantity: 1,
            craft_consider_success: true, // 사용자 요청: 성공률 감안 기본 ON
            craft_consider_critical: false, // 사용자 요청: 대성공 감안 기본 OFF
            cost_calculation: None,
            all_recipes: Vec::new(),
            favorites: HashSet::new(),
            material_batch_count: 100,
            custom_material_durations: HashMap::new(),
            material_sort_mode: MaterialSortMode::SecondaryProfit,
            selected_material_name: None,
            show_ai_connector_popup: false,
            has_shown_ai_connector_popup: false,
            ai_connector_retry_failed: false,
            last_refresh_tick: std::time::Instant::now(),
        };

        app.refresh_data_sync();
        app
    }

    /// 동기적 데이터 갱신
    pub fn refresh_data_sync(&mut self) {
        let cli = self.cli.clone();
        let db = self.db.clone();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                // 1. CLI 상태 확인
                if let Ok(st) = cli.check_status().await {
                    self.cli_connected = st.pipe == "connected";
                } else {
                    self.cli_connected = false;
                }

                // 게임 프로세스(MabinogiMobile)가 실행 중인데도 CLI 미연동 시 AI 커넥터 켜기 안내 팝업 자동 트리거 (최초 1회)
                if !self.cli_connected {
                    if crate::cli::client::is_game_process_running() && !self.has_shown_ai_connector_popup {
                        self.show_ai_connector_popup = true;
                        self.has_shown_ai_connector_popup = true;
                    }
                } else {
                    self.show_ai_connector_popup = false;
                    self.ai_connector_retry_failed = false;
                }

                // 2. 캐릭터, 직업, 전체 보관함(가방+내창고+공용창고) 조회
                if self.cli_connected {
                    if let Ok(info) = cli.get_my_info().await {
                        let realm = info.realm_name.unwrap_or_else(|| "서버".to_string());
                        let lvl = info.level.unwrap_or(0);
                        self.job_name = info.job_name.unwrap_or_else(|| "전투직업".to_string());
                        self.character_info = format!("{} (Lv.{})", realm, lvl);
                    }
                    if let Ok(items) = cli.get_items().await {
                        self.owned_items_cache = items;
                    }
                }

                // 3. 모니터링 품목 목록 조회
                if let Ok(items) = db.get_monitored_items().await {
                    self.monitored_items = items;
                    if !self.monitored_items.contains(&self.selected_item) && !self.monitored_items.is_empty() {
                        self.selected_item = self.monitored_items[0].clone();
                    }
                }

                // 4. 레시피 로드
                if let Ok(recipes) = db.get_recipes(None).await {
                    self.all_recipes = recipes;
                    if self.selected_recipe.is_none() && !self.all_recipes.is_empty() {
                        let job = self.job_name.clone();
                        if let Some(job_rec) = self
                            .all_recipes
                            .iter()
                            .find(|r| is_job_equipment(&job, &r.display_name))
                        {
                            self.selected_recipe = Some(job_rec.clone());
                        } else {
                            self.selected_recipe = Some(self.all_recipes[0].clone());
                        }
                    }
                }

                // 5. 모니터링 품목 + 모든 레시피 장비의 최신 최저가 및 스냅샷 조회 (ZZ 양방향 인덱싱 적용)
                let mut all_target_items = self.monitored_items.clone();
                for r in &self.all_recipes {
                    if !all_target_items.contains(&r.display_name) {
                        all_target_items.push(r.display_name.clone());
                    }
                }
                for name in &all_target_items {
                    if let Ok(snaps) = db.get_snapshot_history(name, 5000).await {
                        if !snaps.is_empty() {
                            if let Some(last) = snaps.last() {
                                self.latest_prices.insert(name.clone(), last.min_unit_price);
                                if name.ends_with("ZZ") {
                                    self.latest_prices.insert(name.trim_end_matches("ZZ").to_string(), last.min_unit_price);
                                } else {
                                    self.latest_prices.insert(format!("{}ZZ", name), last.min_unit_price);
                                }
                            }
                            if name.ends_with("ZZ") {
                                self.snapshots_cache.insert(name.trim_end_matches("ZZ").to_string(), snaps.clone());
                            } else {
                                self.snapshots_cache.insert(format!("{}ZZ", name), snaps.clone());
                            }
                            self.snapshots_cache.insert(name.clone(), snaps);
                        }
                    }
                }

                // 6. 찜(Favorites) 목록 로드
                if let Ok(favs) = db.get_all_favorites().await {
                    self.favorites = favs;
                }

                // 7. 제작 계산 갱신
                self.recalculate_cost();
            });
        });
    }

    /// 거래소 전체 실시간 검색 실행
    pub fn search_market_online(&mut self) {
        let kw = self.market_search_keyword.trim().to_string();
        if kw.is_empty() {
            return;
        }

        self.is_searching_market = true;
        self.search_error_msg = None;
        let cli = self.cli.clone();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                match cli.get_market_prices(&[kw.clone()]).await {
                    Ok(res) => {
                        let mut rows = Vec::new();
                        if let Some(results) = res.results {
                            for q in results {
                                for r in q.rows {
                                    rows.push(r);
                                }
                            }
                        }
                        self.market_search_results = rows;
                    }
                    Err(e) => {
                        self.search_error_msg = Some(format!("검색 실패: {}", e));
                    }
                }
            });
        });
        self.is_searching_market = false;
    }

    /// 모니터링 품목 추가
    pub fn add_item_to_monitored(&mut self, item_name: &str) {
        let db = self.db.clone();
        let name = item_name.to_string();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let _ = db.add_monitored_item(&name, Some("사용자 추가")).await;
            });
        });
        self.selected_item = item_name.to_string();
        self.refresh_data_sync();
    }

    /// 모니터링 품목 제거
    pub fn remove_item_from_monitored(&mut self, item_name: &str) {
        let db = self.db.clone();
        let name = item_name.to_string();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let _ = db.remove_monitored_item(&name).await;
            });
        });
        self.refresh_data_sync();
    }

    /// 제작비용 재계산
    fn recalculate_cost(&mut self) {
        let recipe = match &self.selected_recipe {
            Some(r) => r.clone(),
            None => return,
        };

        let db = self.db.clone();
        let qty = self.craft_quantity as i64;
        let owned_items = self.owned_items_cache.clone();
        let consider_success = self.craft_consider_success;
        let consider_critical = self.craft_consider_critical;

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let calc = CraftCalculator::new(db);
                if let Ok(res) = calc.calculate(&recipe.display_name, qty, &owned_items, consider_success, consider_critical).await {
                    self.cost_calculation = Some(res);
                }
            });
        });
    }

    /// 관심 장비/아이템 찜(Favorites) 여부 확인 (ZZ 접미사 양방향 대응)
    pub fn is_favorite(&self, item_name: &str) -> bool {
        self.favorites.contains(item_name)
            || self.favorites.contains(&format!("{}ZZ", item_name))
            || self.favorites.contains(item_name.trim_end_matches("ZZ"))
    }

    /// 관심 장비/아이템 찜(Favorites) 토글 (메모리 및 DB 즉시 반영)
    pub fn toggle_favorite(&mut self, item_name: &str) {
        let is_fav = self.is_favorite(item_name);
        let name = item_name.to_string();
        let db = self.db.clone();

        if is_fav {
            self.favorites.remove(&name);
            self.favorites.remove(&format!("{}ZZ", name));
            self.favorites.remove(name.trim_end_matches("ZZ"));
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    let _ = db.remove_favorite(&name).await;
                    let _ = db.remove_favorite(&format!("{}ZZ", name)).await;
                    let _ = db.remove_favorite(name.trim_end_matches("ZZ")).await;
                });
            });
        } else {
            self.favorites.insert(name.clone());
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    let _ = db.add_favorite(&name).await;
                });
            });
        }
    }

    /// 현재 선택된 그룹의 아이템 목록 반환
    fn get_group_items(&self, group: ItemGroup) -> Vec<String> {
        match group {
            ItemGroup::Favorites => {
                let mut list: Vec<String> = self.favorites.iter().cloned().collect();
                list.sort();
                list
            }
            ItemGroup::All => self.monitored_items.clone(),
            ItemGroup::HighValue => GROUP_HIGH_VALUE.iter().map(|s| s.to_string()).collect(),
            ItemGroup::MagicStones => GROUP_MAGIC_STONES.iter().map(|s| s.to_string()).collect(),
            ItemGroup::SoulStones => GROUP_SOUL_STONES.iter().map(|s| s.to_string()).collect(),
            ItemGroup::Boxes => GROUP_BOXES.iter().map(|s| s.to_string()).collect(),
            ItemGroup::Bags => GROUP_BAGS.iter().map(|s| s.to_string()).collect(),
            ItemGroup::Remnant => {
                let job = &self.job_name;
                let items: Vec<String> = self.all_recipes
                    .iter()
                    .filter(|r| r.display_name.contains("잔영") && (job == "미연동" || is_job_equipment(job, &r.display_name)))
                    .map(|r| r.display_name.clone())
                    .collect();
                if items.is_empty() {
                    self.monitored_items.iter().filter(|s| s.contains("잔영")).cloned().collect()
                } else {
                    items
                }
            }
            ItemGroup::Abyss => {
                let job = &self.job_name;
                let items: Vec<String> = self.all_recipes
                    .iter()
                    .filter(|r| r.display_name.contains("해연") && (job == "미연동" || is_job_equipment(job, &r.display_name)))
                    .map(|r| r.display_name.clone())
                    .collect();
                if items.is_empty() {
                    self.monitored_items.iter().filter(|s| s.contains("해연")).cloned().collect()
                } else {
                    items
                }
            }
            ItemGroup::MyJob => {
                let job = &self.job_name;
                self.all_recipes
                    .iter()
                    .filter(|r| is_job_equipment(job, &r.display_name))
                    .map(|r| r.display_name.clone())
                    .collect()
            }
        }
    }

    /// 캐시된 최신 가격 조회 (ZZ 양방향 호환)
    pub fn get_cached_price(&self, name: &str) -> Option<i64> {
        if let Some(p) = self.latest_prices.get(name) {
            return Some(*p);
        }
        if name.ends_with("ZZ") {
            let alt = name.trim_end_matches("ZZ");
            if let Some(p) = self.latest_prices.get(alt) {
                return Some(*p);
            }
        } else {
            let alt = format!("{}ZZ", name);
            if let Some(p) = self.latest_prices.get(&alt) {
                return Some(*p);
            }
        }
        None
    }

    /// 조회 기간(period_days)에 맞춰 필터링된 스냅샷 반환 (ZZ 양방향 호환, 0은 전체 기간)
    fn get_filtered_snapshots<'a>(&'a self, item_name: &str) -> Vec<&'a SnapshotRecord> {
        let snaps = if let Some(s) = self.snapshots_cache.get(item_name) {
            s
        } else if item_name.ends_with("ZZ") {
            let alt = item_name.trim_end_matches("ZZ");
            match self.snapshots_cache.get(alt) {
                Some(s) => s,
                None => return Vec::new(),
            }
        } else {
            let alt = format!("{}ZZ", item_name);
            match self.snapshots_cache.get(&alt) {
                Some(s) => s,
                None => return Vec::new(),
            }
        };

        if self.period_days == 0 {
            return snaps.iter().collect();
        }

        let now = chrono::Local::now().naive_local();
        let max_age_secs = (self.period_days as i64) * 86400;

        snaps
            .iter()
            .filter(|s| {
                if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(&s.scraped_at, "%Y-%m-%d %H:%M:%S") {
                    let diff = (now - parsed).num_seconds();
                    diff <= max_age_secs
                } else {
                    true
                }
            })
            .collect()
    }

    /// 기간 내 (기간 최저가, 기간 최고가) 계산
    fn get_period_min_max(&self, item_name: &str) -> (Option<i64>, Option<i64>) {
        let filtered = self.get_filtered_snapshots(item_name);
        if filtered.is_empty() {
            (None, None)
        } else {
            let min_p = filtered.iter().map(|s| s.min_unit_price).min();
            let max_p = filtered.iter().map(|s| s.min_unit_price).max();
            (min_p, max_p)
        }
    }
}

/// 한글 맑은 고딕 폰트 등록 함수
fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let font_path = std::path::Path::new(r"C:\Windows\Fonts\malgun.ttf");
    if font_path.exists() {
        if let Ok(font_data) = std::fs::read(font_path) {
            fonts.font_data.insert(
                "malgun".to_owned(),
                egui::FontData::from_owned(font_data),
            );

            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "malgun".to_owned());

            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(0, "malgun".to_owned());

            ctx.set_fonts(fonts);
        }
    }
}

impl eframe::App for MobiMarketApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.last_refresh_tick.elapsed().as_secs() >= 2 {
            self.last_refresh_tick = std::time::Instant::now();
            let cur_interval = self.interval_minutes as u64 * 60;
            self.collector
                .interval_secs
                .store(cur_interval, Ordering::Relaxed);
        }

        // 상단 헤더 영역
        egui::TopBottomPanel::top("top_header").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading(
                    RichText::new("MobiMarket")
                        .size(20.0)
                        .strong()
                        .color(Color32::from_rgb(250, 180, 40)),
                );

                // 버전명 배지 (로고 바로 우측 눈에 띄는 다크 배지)
                ui.label(
                    RichText::new(format!(" {} ", APP_VERSION))
                        .size(11.0)
                        .strong()
                        .color(Color32::from_rgb(210, 225, 245))
                        .background_color(Color32::from_rgb(45, 55, 70)),
                );

                ui.add_space(8.0);

                // CLI 연결 상태 배지를 로고 바로 옆에 배치하여 좌우 균형 및 가로 공간 극대화
                if self.cli_connected {
                    ui.label(
                        RichText::new(format!(
                            "● CLI 연동됨 ({}, {})",
                            self.character_info, self.job_name
                        ))
                        .color(Color32::from_rgb(100, 230, 120))
                        .strong(),
                    );
                } else {
                    if ui.small_button(RichText::new("[AI 커넥터 켜기 안내]").color(Color32::from_rgb(255, 210, 80))).clicked() {
                        self.show_ai_connector_popup = true;
                    }
                    ui.label(
                        RichText::new("● 게임 미실행 / CLI 미연동")
                            .color(Color32::from_rgb(250, 100, 100))
                            .strong(),
                    );
                }

                // 우측 컨트롤 영역: 콤팩트 ComboBox로 통일하여 겹침 원천 차단
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let is_collecting = self.collector.is_collecting.load(Ordering::Relaxed);
                    if is_collecting {
                        ui.add_enabled(
                            false,
                            egui::Button::new(
                                RichText::new("최저가 수집 중...").color(Color32::YELLOW),
                            ),
                        );
                    } else if ui
                        .button(
                            RichText::new("최저가 지금 수집")
                                .color(Color32::WHITE)
                                .strong(),
                        )
                        .clicked()
                    {
                        if !self.cli_connected {
                            self.show_ai_connector_popup = true;
                        } else {
                            self.collector.request_immediate_run();
                            self.refresh_data_sync();
                        }
                    }

                    ui.separator();

                    // 남은 시간 카운트다운 (남은 분/초 명확화)
                    let rem_secs = self.collector.remaining_secs.load(Ordering::Relaxed);
                    let min = rem_secs / 60;
                    let sec = rem_secs % 60;
                    ui.label(
                        RichText::new(format!("다음 수집: {:02}분 {:02}초 후", min, sec))
                            .size(12.0)
                            .color(Color32::from_rgb(160, 200, 255)),
                    );

                    ui.separator();

                    // 자동 주기 ComboBox (슬라이더 대신 콤팩트 드롭다운)
                    let prev_val = self.interval_minutes;
                    egui::ComboBox::from_id_salt("header_interval_combo")
                        .selected_text(format!("주기: {}분", self.interval_minutes))
                        .width(76.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.interval_minutes, 1, "1분");
                            ui.selectable_value(&mut self.interval_minutes, 5, "5분");
                            ui.selectable_value(&mut self.interval_minutes, 10, "10분");
                            ui.selectable_value(&mut self.interval_minutes, 30, "30분");
                            ui.selectable_value(&mut self.interval_minutes, 60, "60분");
                        });
                    if prev_val != self.interval_minutes {
                        let new_sec = self.interval_minutes as u64 * 60;
                        self.collector
                            .interval_secs
                            .store(new_sec, Ordering::Relaxed);
                    }

                    ui.separator();

                    // 조회 기간 ComboBox (슬라이더 + 6개 버튼 대신 콤팩트 드롭다운)
                    let period_text = if self.period_days == 0 {
                        "기간: 전체".to_string()
                    } else {
                        format!("기간: {}일", self.period_days)
                    };
                    egui::ComboBox::from_id_salt("header_period_combo")
                        .selected_text(period_text)
                        .width(76.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.period_days, 1, "최근 1일");
                            ui.selectable_value(&mut self.period_days, 3, "최근 3일");
                            ui.selectable_value(&mut self.period_days, 7, "최근 7일");
                            ui.selectable_value(&mut self.period_days, 14, "최근 14일");
                            ui.selectable_value(&mut self.period_days, 30, "최근 30일");
                            ui.selectable_value(&mut self.period_days, 0, "전체 기간");
                        });
                });
            });

            ui.add_space(6.0);
            ui.separator();

            // 메인 탭바 (문자 깨짐 없는 깔끔한 텍스트 탭)
            ui.horizontal(|ui| {
                ui.selectable_value(
                    &mut self.active_tab,
                    MainTab::MarketPrices,
                    RichText::new("  [시세] 주요 아이템 시세  ").size(15.0).strong(),
                );
                ui.selectable_value(
                    &mut self.active_tab,
                    MainTab::CraftCalculator,
                    RichText::new("  [제작] 제작 아이템 계산기  ").size(15.0).strong(),
                );
                ui.selectable_value(
                    &mut self.active_tab,
                    MainTab::TimingAnalysis,
                    RichText::new("  ⭐ [플래너] 내 찜 & 스펙업 견적  ")
                        .size(15.0)
                        .strong()
                        .color(Color32::from_rgb(255, 215, 80)),
                );
                ui.selectable_value(
                    &mut self.active_tab,
                    MainTab::MaterialProfit,
                    RichText::new("  💰 [데카벌이] 제작재료 데카벌이  ")
                        .size(15.0)
                        .strong()
                        .color(Color32::from_rgb(120, 230, 140)),
                );
            });
            ui.add_space(4.0);
        });

        // 메인 콘텐츠 패널
        egui::CentralPanel::default().show(ctx, |ui| match self.active_tab {
            MainTab::MarketPrices => self.view_market_prices(ui),
            MainTab::CraftCalculator => self.view_craft_calculator(ui),
            MainTab::TimingAnalysis => self.view_timing_analysis(ui),
            MainTab::MaterialProfit => self.view_material_profit_calculator(ui),
        });

        // AI 커넥터 비활성화 안내 팝업 모달
        self.render_ai_connector_popup(ctx);

        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}

impl MobiMarketApp {
    /// AI 커넥터 비활성화 안내 팝업 모달 렌더링
    fn render_ai_connector_popup(&mut self, ctx: &egui::Context) {
        if !self.show_ai_connector_popup {
            return;
        }

        egui::Window::new("마비노기 모바일 AI 커넥터 활성화 안내")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("⚠ [AI 커넥터] 설정이 꺼져 있습니다!")
                            .size(16.0)
                            .strong()
                            .color(Color32::from_rgb(255, 190, 60)),
                    );
                });

                ui.add_space(8.0);
                ui.label(
                    RichText::new("게임 내에서 '마비노기 모바일 AI 커넥터'가 활성화되어 있어야 거래소 실시간 시세 및 내 가방/창고 아이템을 조회할 수 있습니다.")
                        .size(13.0)
                        .color(Color32::WHITE),
                );

                ui.add_space(10.0);

                egui::Frame::none()
                    .fill(Color32::from_rgb(30, 35, 45))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(60, 75, 95)))
                    .inner_margin(egui::Margin::symmetric(14.0, 10.0))
                    .rounding(6.0)
                    .show(ui, |ui| {
                        ui.label(RichText::new("▶ 게임 내 활성화 방법").strong().color(Color32::from_rgb(120, 220, 255)));
                        ui.add_space(4.0);
                        ui.label("1. 게임 화면 우측 상단 메뉴 또는 ESC 키 -> [환경설정]");
                        ui.label("2. 좌측 메뉴 [기타] 또는 [AI 제어] 탭 클릭");
                        ui.label("3. [마비노기 모바일 AI 커넥터] 스위치를 켬(ON)으로 설정");
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("※ 일주일간 미사용 시 게임에서 자동으로 비활성화되므로 다시 켜주셔야 합니다.")
                                .size(11.0)
                                .color(Color32::GRAY),
                        );
                    });

                if self.ai_connector_retry_failed {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("⚠ 아직 연결되지 않았습니다. 게임 내에서 커넥터를 ON으로 켠 후 다시 시도해 주세요.")
                            .color(Color32::LIGHT_RED)
                            .size(12.0),
                    );
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("  다시 연결 시도  ").strong().color(Color32::WHITE)).clicked() {
                        self.refresh_data_sync();
                        if self.cli_connected {
                            self.show_ai_connector_popup = false;
                            self.ai_connector_retry_failed = false;
                        } else {
                            self.ai_connector_retry_failed = true;
                        }
                    }

                    if ui.button("  닫기  ").clicked() {
                        self.show_ai_connector_popup = false;
                        self.ai_connector_retry_failed = false;
                    }
                });
                ui.add_space(6.0);
            });
    }
}

impl MobiMarketApp {
    /// 탭 1: 주요 아이템 시세 뷰
    fn view_market_prices(&mut self, ui: &mut egui::Ui) {
        let total_height = ui.available_height();
        let total_width = ui.available_width();
        let left_panel_width = (total_width * 0.48).clamp(580.0, 720.0);

        ui.horizontal(|ui| {
            // 좌측 패널 (전체 너비의 약 50% 수준으로 대폭 확장하여 뷰 밸런스 및 품목명 가독성 확보)
            ui.vertical(|ui| {
                ui.set_width(left_panel_width);

                // 1. 그룹 선택 버튼바
                ui.label(RichText::new("품목 그룹 선택").size(12.0).color(Color32::GRAY));
                ui.horizontal_wrapped(|ui| {
                    for grp in &[
                        ItemGroup::Favorites,
                        ItemGroup::All,
                        ItemGroup::HighValue,
                        ItemGroup::MagicStones,
                        ItemGroup::SoulStones,
                        ItemGroup::Boxes,
                        ItemGroup::Bags,
                        ItemGroup::Remnant,
                        ItemGroup::Abyss,
                        ItemGroup::MyJob,
                    ] {
                        if ui
                            .selectable_label(self.selected_group == *grp, grp.label())
                            .clicked()
                        {
                            self.selected_group = *grp;
                            let items = self.get_group_items(*grp);
                            if let Some(first) = items.first() {
                                self.selected_item = first.clone();
                            }
                        }
                    }
                });

                ui.add_space(6.0);
                ui.separator();

                // 3. 거래소 전체 실시간 검색창
                ui.label(
                    RichText::new("거래소 전체 아이템 검색 & 모니터링 추가")
                        .size(12.0)
                        .strong()
                        .color(Color32::from_rgb(120, 220, 255)),
                );

                ui.horizontal(|ui| {
                    let text_edit = ui.text_edit_singleline(&mut self.market_search_keyword);
                    if text_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.search_market_online();
                    }
                    if ui.button("검색").clicked() {
                        self.search_market_online();
                    }
                });

                // 검색 결과가 있는 경우 표시
                if !self.market_search_results.is_empty() {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "검색 결과: {}건",
                                    self.market_search_results.len()
                                ))
                                .size(11.0)
                                .color(Color32::YELLOW),
                            );
                            if ui.small_button("닫기").clicked() {
                                self.market_search_results.clear();
                            }
                        });

                        egui::ScrollArea::vertical()
                            .id_salt("market_search_results_scroll")
                            .drag_to_scroll(false)
                            .auto_shrink([false, false])
                            .max_height(180.0)
                            .show(ui, |ui| {
                                let mut item_to_add: Option<String> = None;
                                for row in &self.market_search_results {
                                    let is_monitored = self.monitored_items.contains(&row.display_name);
                                    let price_text = if let Some(p) = row.min_unit_price {
                                        format!("{} 데카", format_number(p))
                                    } else {
                                        "품절".to_string()
                                    };

                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(&row.display_name).size(12.0));
                                        ui.label(
                                            RichText::new(price_text)
                                                .size(11.0)
                                                .color(Color32::from_rgb(250, 190, 80)),
                                        );

                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if is_monitored {
                                                    ui.label(
                                                        RichText::new("[모니터링 중]")
                                                            .size(10.0)
                                                            .color(Color32::GREEN),
                                                    );
                                                } else if ui.small_button("+ 추가").clicked() {
                                                    item_to_add = Some(row.display_name.clone());
                                                }
                                            },
                                        );
                                    });
                                }

                                if let Some(name) = item_to_add {
                                    self.add_item_to_monitored(&name);
                                }
                            });
                    });
                }

                if let Some(err) = &self.search_error_msg {
                    ui.label(RichText::new(err).size(11.0).color(Color32::LIGHT_RED));
                }

                ui.add_space(4.0);
                ui.separator();

                // 3. 필터링된 모니터링 아이템 목록 (440px HTS 4열 레이아웃)
                let items_to_show = self.get_group_items(self.selected_group);

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} 목록 ({}개)",
                            self.selected_group.label(),
                            items_to_show.len()
                        ))
                        .size(12.0)
                        .color(Color32::LIGHT_BLUE),
                    );

                    let period_label = if self.period_days == 0 {
                        "전체 기간".to_string()
                    } else {
                        format!("최근 {}일", self.period_days)
                    };
                    ui.label(
                        RichText::new(format!("({})", period_label))
                            .size(11.0)
                            .color(Color32::GRAY),
                    );
                });

                ui.add_space(2.0);

                // HTS 4열 고정 컬럼 헤더 라인 (데이터 행과 1:1 수직 칼정렬)
                let h_available_w = ui.available_width();
                let (h_rect, _) = ui.allocate_exact_size(
                    egui::vec2(h_available_w, 20.0),
                    egui::Sense::hover(),
                );
                let h_base_x = h_rect.min.x;
                let h_center_y = h_rect.center().y;

                // 컬럼 1: 찜 & 품목명 (여유 너비 ~350px 확보)
                ui.painter().text(
                    egui::pos2(h_base_x + 6.0, h_center_y),
                    egui::Align2::LEFT_CENTER,
                    "찜",
                    egui::FontId::proportional(11.0),
                    Color32::from_rgb(255, 215, 80),
                );
                ui.painter().text(
                    egui::pos2(h_base_x + 28.0, h_center_y),
                    egui::Align2::LEFT_CENTER,
                    "품목명",
                    egui::FontId::proportional(11.0),
                    Color32::GRAY,
                );
                // 컬럼 2: 현재가
                ui.painter().text(
                    egui::pos2(h_base_x + 360.0, h_center_y),
                    egui::Align2::RIGHT_CENTER,
                    "현재가",
                    egui::FontId::proportional(11.0),
                    Color32::from_rgb(250, 190, 80),
                );
                // 컬럼 3: 기간 최저
                ui.painter().text(
                    egui::pos2(h_base_x + 455.0, h_center_y),
                    egui::Align2::RIGHT_CENTER,
                    "기간 최저",
                    egui::FontId::proportional(11.0),
                    Color32::from_rgb(100, 210, 255),
                );
                // 컬럼 4: 기간 최고
                ui.painter().text(
                    egui::pos2(h_base_x + 550.0, h_center_y),
                    egui::Align2::RIGHT_CENTER,
                    "기간 최고",
                    egui::FontId::proportional(11.0),
                    Color32::from_rgb(255, 120, 120),
                );
                ui.separator();

                let mut item_to_remove: Option<String> = None;
                let mut item_to_toggle_fav: Option<String> = None;

                let upper_overhead = if self.market_search_results.is_empty() { 155.0 } else { 355.0 };
                let list_height = (total_height - upper_overhead).max(430.0);

                egui::ScrollArea::vertical()
                    .id_salt("market_items_list_scroll")
                    .drag_to_scroll(false)
                    .auto_shrink([false, false])
                    .min_scrolled_height(list_height)
                    .max_height(list_height)
                    .show(ui, |ui| {
                        for name in items_to_show {
                            let is_selected = self.selected_item == name;

                            // 1. 현재 최저가 (거래소 등록 최저가 - D 제외 숫자만, ZZ 호환)
                            let cur_price_str = if let Some(p) = self.get_cached_price(&name) {
                                format_number(p)
                            } else {
                                "-".to_string()
                            };

                            // 2. 조회 기간 내 최저가 / 최고가 산출 (D 제외 숫자만)
                            let (p_min, p_max) = self.get_period_min_max(&name);
                            let min_price_str = if let Some(p) = p_min {
                                format_number(p)
                            } else {
                                "-".to_string()
                            };
                            let max_price_str = if let Some(p) = p_max {
                                format_number(p)
                            } else {
                                "-".to_string()
                            };

                            let is_deletable = self.selected_group == ItemGroup::All
                                && !crate::config::DEFAULT_MONITORED_NAMES.contains(&name.as_str());

                            ui.push_id(&name, |ui| {
                                let row_height = 26.0;
                                let available_w = ui.available_width();
                                let (rect, resp) = ui.allocate_exact_size(
                                    egui::vec2(available_w, row_height),
                                    egui::Sense::click(),
                                );
                                let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);

                                let base_x = rect.min.x;
                                let center_y = rect.center().y;

                                let mut del_clicked = false;
                                let mut fav_clicked = false;

                                // 0. 찜 토글 버튼 (base_x + 2.0 ~ base_x + 24.0)
                                let is_fav = self.is_favorite(&name);
                                let fav_rect = egui::Rect::from_min_max(
                                    egui::pos2(base_x + 2.0, rect.min.y + 2.0),
                                    egui::pos2(base_x + 24.0, rect.max.y - 2.0),
                                );
                                let fav_resp = ui
                                    .interact(fav_rect, ui.id().with("fav_btn"), egui::Sense::click())
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .on_hover_text(if is_fav { "찜 해제" } else { "관심 장비로 찜하기" });
                                if fav_resp.clicked() {
                                    fav_clicked = true;
                                    item_to_toggle_fav = Some(name.clone());
                                }

                                if is_deletable {
                                    let del_rect = egui::Rect::from_min_max(
                                        egui::pos2(base_x + 558.0, rect.min.y + 3.0),
                                        egui::pos2(base_x + 582.0, rect.max.y - 3.0),
                                    );
                                    let del_resp = ui
                                        .interact(del_rect, ui.id().with("del_btn"), egui::Sense::click())
                                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                                    let del_bg = if del_resp.hovered() {
                                        Color32::from_rgb(180, 50, 50)
                                    } else {
                                        Color32::from_rgb(70, 75, 85)
                                    };
                                    ui.painter().rect_filled(del_rect, 3.0, del_bg);
                                    ui.painter().text(
                                        del_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        "삭제",
                                        egui::FontId::proportional(10.0),
                                        Color32::WHITE,
                                    );
                                    if del_resp.clicked() {
                                        del_clicked = true;
                                        item_to_remove = Some(name.clone());
                                    }
                                }

                                if !del_clicked && !fav_clicked && resp.clicked() {
                                    self.selected_item = name.clone();
                                }

                                // 행 배경 하이라이트 (선택 시 진한 파랑, 마우스 오버 시 소프트 하이라이트)
                                let bg_color = if is_selected {
                                    Color32::from_rgb(25, 70, 110)
                                } else if resp.hovered() {
                                    Color32::from_rgb(45, 50, 60)
                                } else {
                                    Color32::TRANSPARENT
                                };
                                ui.painter().rect_filled(rect, 4.0, bg_color);

                                // 찜 아이콘 페인팅 (골드 별 or 어두운 별)
                                let (fav_icon, fav_color) = if is_fav {
                                    ("★", Color32::from_rgb(255, 215, 0))
                                } else {
                                    ("☆", Color32::from_rgb(110, 120, 135))
                                };
                                ui.painter().text(
                                    fav_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    fav_icon,
                                    egui::FontId::proportional(13.0),
                                    fav_color,
                                );

                                // 컬럼 1: 좌측 품목명 텍스트 페인팅 (여유 너비 ~350px로 20자 이상 긴 이름도 가격과 겹치지 않음)
                                let text_color = if is_selected {
                                    Color32::WHITE
                                } else {
                                    Color32::from_rgb(210, 220, 230)
                                };
                                ui.painter().text(
                                    egui::pos2(base_x + 28.0, center_y),
                                    egui::Align2::LEFT_CENTER,
                                    &name,
                                    egui::FontId::proportional(12.0),
                                    text_color,
                                );

                                // 컬럼 2: 현재가 (골드)
                                let cur_price_color = Color32::from_rgb(250, 190, 80);
                                ui.painter().text(
                                    egui::pos2(base_x + 360.0, center_y),
                                    egui::Align2::RIGHT_CENTER,
                                    cur_price_str,
                                    egui::FontId::proportional(11.5),
                                    cur_price_color,
                                );

                                // 컬럼 3: 기간 최저가 (청량한 민트/하늘)
                                let min_price_color = Color32::from_rgb(100, 210, 255);
                                ui.painter().text(
                                    egui::pos2(base_x + 455.0, center_y),
                                    egui::Align2::RIGHT_CENTER,
                                    min_price_str,
                                    egui::FontId::proportional(11.5),
                                    min_price_color,
                                );

                                // 컬럼 4: 기간 최고가 (산뜻한 로즈/오렌지 - is_deletable 상관없이 항상 base_x + 550.0 고정)
                                let max_price_color = Color32::from_rgb(255, 120, 120);
                                ui.painter().text(
                                    egui::pos2(base_x + 550.0, center_y),
                                    egui::Align2::RIGHT_CENTER,
                                    max_price_str,
                                    egui::FontId::proportional(11.5),
                                    max_price_color,
                                );

                                ui.add_space(2.0);
                            });
                        }
                    });

                if let Some(del_name) = item_to_remove {
                    self.remove_item_from_monitored(&del_name);
                }
                if let Some(fav_item) = item_to_toggle_fav {
                    self.toggle_favorite(&fav_item);
                }
            });

            ui.separator();

            // 우측 메인 패널: 다중 라인 비교 그래프 (egui_plot) + 상세 요약
            ui.vertical(|ui| {
                let current_group = self.selected_group;
                let is_group_mode = current_group != ItemGroup::All;
                let group_items = self.get_group_items(current_group);

                // 1. 그룹 모드일 때 조회 기간 내 변동량(최고가 - 최저가) 기준 내림차순 정렬 및 상위 5개 선별
                let mut ranked_items: Vec<(String, i64, i64)> = Vec::new(); // (품목명, 변동량, 최신가)
                if is_group_mode {
                    for item_name in &group_items {
                        let snaps = self.get_filtered_snapshots(item_name);
                        if !snaps.is_empty() {
                            let min_p = snaps.iter().map(|s| s.min_unit_price).min().unwrap_or(0);
                            let max_p = snaps.iter().map(|s| s.min_unit_price).max().unwrap_or(0);
                            let vol = max_p - min_p;
                            let latest = snaps.last().map(|s| s.min_unit_price).unwrap_or(0);
                            ranked_items.push((item_name.clone(), vol, latest));
                        }
                    }
                    // 변동량 내림차순 정렬 (동률 시 최신 가격 내림차순)
                    ranked_items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.2.cmp(&a.2)));
                }

                let is_truncated = is_group_mode && group_items.len() > 5;
                let display_items: Vec<String> = if is_group_mode {
                    if ranked_items.is_empty() {
                        group_items.iter().take(5).cloned().collect()
                    } else {
                        ranked_items.iter().take(5).map(|(name, _, _)| name.clone()).collect()
                    }
                } else {
                    vec![self.selected_item.clone()]
                };

                // 최대 변동폭 품목 식별 (강조 하이라이트용)
                let most_volatile_item = ranked_items.first().and_then(|(name, vol, _)| if *vol > 0 { Some(name.clone()) } else { None });

                let period_hint = if self.period_days == 0 {
                    "(전체 기간)".to_string()
                } else {
                    format!("(최근 {}일간)", self.period_days)
                };

                // 2. 상단 헤더 텍스트 렌더링
                ui.horizontal(|ui| {
                    if is_group_mode {
                        let sub_hint = if is_truncated {
                            format!("(변동량 상위 5개 품목 / 총 {}개 중) {}", group_items.len(), period_hint)
                        } else {
                            format!("(동일 카테고리 품목 간 다중 라인 상호 비교) {}", period_hint)
                        };
                        ui.heading(
                            RichText::new(format!("{} 비교 분석", current_group.label()))
                                .size(18.0)
                                .color(Color32::WHITE),
                        );
                        ui.label(RichText::new(sub_hint).color(Color32::GRAY));
                    } else {
                        ui.heading(
                            RichText::new(&self.selected_item)
                                .size(18.0)
                                .color(Color32::WHITE),
                        );
                        if let Some(price) = self.get_cached_price(&self.selected_item) {
                            ui.label(
                                RichText::new(format!("현재 최저가: {} 데카", format_number(price)))
                                    .size(16.0)
                                    .strong()
                                    .color(Color32::from_rgb(250, 190, 80)),
                            );
                        }
                        ui.label(RichText::new(period_hint).color(Color32::GRAY));
                    }
                });

                ui.add_space(6.0);

                // 3. 변동량 가시성 극대화를 위한 Y축 최적 하한/상한 여백 계산 (선택 기간 필터 적용)
                let mut plot_min_price = i64::MAX;
                let mut plot_max_price = 0_i64;
                let mut has_plot_data = false;

                for name in &display_items {
                    let snaps = self.get_filtered_snapshots(name);
                    for s in snaps {
                        has_plot_data = true;
                        if s.min_unit_price < plot_min_price {
                            plot_min_price = s.min_unit_price;
                        }
                        if s.min_unit_price > plot_max_price {
                            plot_max_price = s.min_unit_price;
                        }
                    }
                }

                let (y_include_min, y_include_max) = if has_plot_data && plot_min_price <= plot_max_price {
                    let spread = (plot_max_price - plot_min_price) as f64;
                    let margin = (spread * 0.20).max((plot_max_price as f64) * 0.05).max(10.0);
                    let y_min = ((plot_min_price as f64) - margin).max(0.0);
                    let y_max = (plot_max_price as f64) + margin;
                    (y_min, y_max)
                } else {
                    (0.0, 1000.0)
                };

                // 4. 그래프 (Plot) 영역: 720p 최적 높이(240px) + 자동 뷰 리셋 + 변동량 극대화 스케일
                let plot_height = 240.0;
                let plot_id = if is_group_mode {
                    format!("market_plot_group_{:?}_{}", current_group, self.period_days)
                } else {
                    format!("market_plot_item_{}_{}", self.selected_item, self.period_days)
                };

                Plot::new(plot_id)
                    .legend(Legend::default().position(egui_plot::Corner::LeftTop))
                    .height(plot_height)
                    .show_axes([true, true])
                    .show_grid(true)
                    .include_y(y_include_min) // 변동량 가시성을 살리는 최적 하한선 (음수 방지)
                    .include_y(y_include_max) // 변동량 가시성을 살리는 최적 상한선
                    .clamp_grid(true) // 그리드가 마이너스로 스크롤/줌 되지 않도록 제약
                    .y_axis_formatter(|mark, _range| {
                        if mark.value < 0.0 {
                            "0".to_string()
                        } else {
                            format_number(mark.value as i64)
                        }
                    })
                    .show(ui, |plot_ui| {
                        let colors = [
                            Color32::from_rgb(255, 180, 50),  // Gold
                            Color32::from_rgb(80, 200, 255),  // Sky Blue
                            Color32::from_rgb(120, 240, 140), // Light Green
                            Color32::from_rgb(255, 120, 150), // Coral Pink
                            Color32::from_rgb(210, 140, 255), // Purple
                        ];

                        if is_group_mode {
                            // 상위 5개 품목만 렌더링 (범례 넘침 완전 방지)
                            for (idx, item_name) in display_items.iter().enumerate() {
                                let snaps = self.get_filtered_snapshots(item_name);
                                if !snaps.is_empty() {
                                    let points: PlotPoints = snaps
                                        .iter()
                                        .enumerate()
                                        .map(|(i, s)| [i as f64, s.min_unit_price as f64])
                                        .collect();

                                    let is_highest_vol = most_volatile_item.as_ref() == Some(item_name);
                                    let line_width = if is_highest_vol { 3.5_f32 } else { 1.8_f32 };
                                    let color = if is_highest_vol {
                                        Color32::from_rgb(255, 230, 80) // 강조: 밝은 네온 골드
                                    } else {
                                        colors[idx % colors.len()]
                                    };

                                    // 참조표(Legend)에 품목명 + 현재 최저가 동시 표기
                                    let price_str = if let Some(p) = self.get_cached_price(item_name) {
                                        format!("{} 데카", format_number(p))
                                    } else {
                                        "시세없음".to_string()
                                    };

                                    let label_name = if is_highest_vol {
                                        format!("{} ({}) [변동 최대!]", item_name, price_str)
                                    } else {
                                        format!("{} ({})", item_name, price_str)
                                    };

                                    plot_ui.line(
                                        Line::new(points)
                                            .name(label_name)
                                            .color(color)
                                            .width(line_width),
                                    );
                                }
                            }
                        } else {
                            let snaps = self.get_filtered_snapshots(&self.selected_item);
                            if !snaps.is_empty() {
                                let points: PlotPoints = snaps
                                    .iter()
                                    .enumerate()
                                    .map(|(i, s)| [i as f64, s.min_unit_price as f64])
                                    .collect();

                                let price_str = if let Some(p) = self.get_cached_price(&self.selected_item) {
                                    format!("{} 데카", format_number(p))
                                } else {
                                    "시세없음".to_string()
                                };
                                let label_name = format!("{} ({})", self.selected_item, price_str);

                                plot_ui.line(
                                    Line::new(points)
                                        .name(label_name)
                                        .color(Color32::from_rgb(255, 190, 60))
                                        .width(3.0_f32),
                                );
                            }
                        }
                    });

                ui.add_space(6.0);
                ui.separator();

                // 하단: 최근 스냅샷 히스토리 테이블 (ZZ 호환 get_filtered_snapshots 적용)
                ui.label(
                    RichText::new(format!("최근 최저가 수집 기록 - {}", self.selected_item))
                        .size(13.0)
                        .strong(),
                );

                let snaps_list = self.get_filtered_snapshots(&self.selected_item);
                if !snaps_list.is_empty() {
                    let history_height = (total_height - plot_height - 90.0).max(220.0);
                    egui::ScrollArea::vertical()
                        .id_salt("market_history_table_scroll")
                        .drag_to_scroll(false)
                        .auto_shrink([false, false])
                        .min_scrolled_height(history_height)
                        .max_height(history_height)
                        .show(ui, |ui| {
                            egui::Grid::new("snapshot_history_grid")
                                .striped(true)
                                .min_col_width(120.0)
                                .show(ui, |ui| {
                                    ui.label(RichText::new("수집 일시").strong().color(Color32::GRAY));
                                    ui.label(RichText::new("최저가 (데카)").strong().color(Color32::GRAY));
                                    ui.label(RichText::new("재고 상태").strong().color(Color32::GRAY));
                                    ui.end_row();

                                    for s in snaps_list.iter().rev().take(15) {
                                        let stock_str = if s.in_stock { "재고있음" } else { "품절" };
                                        let stock_color = if s.in_stock {
                                            Color32::from_rgb(120, 220, 140)
                                        } else {
                                            Color32::LIGHT_RED
                                        };

                                        ui.label(&s.scraped_at);
                                        ui.label(
                                            RichText::new(format_number(s.min_unit_price))
                                                .color(Color32::from_rgb(255, 200, 80))
                                                .strong(),
                                        );
                                        ui.label(RichText::new(stock_str).color(stock_color));
                                        ui.end_row();
                                    }
                                });
                        });
                }
            });
        });
    }

    /// 탭 2: 제작 아이템 계산기 뷰
    fn view_craft_calculator(&mut self, ui: &mut egui::Ui) {
        let total_height = ui.available_height();

        ui.horizontal(|ui| {
            // 좌측 패널 (너비 340px)
            ui.vertical(|ui| {
                ui.set_width(340.0);

                // 1. 현재 직업 필터 체크박스 (자동 감지된 직업명 표시)
                let prev_job_filter = self.job_only_filter;
                ui.checkbox(
                    &mut self.job_only_filter,
                    RichText::new(format!("내 직업({}) 전용 장비만 보기", self.job_name))
                        .color(Color32::from_rgb(130, 220, 150))
                        .strong(),
                );
                if prev_job_filter != self.job_only_filter {
                    self.select_first_filtered_recipe();
                }

                ui.add_space(4.0);

                // 2. 분류 필터 ([전체], [해연], [잔영] - 사용자 요청: '등급'이 아닌 '분류'로 변경)
                ui.horizontal(|ui| {
                    ui.label(RichText::new("분류:").size(11.0).color(Color32::GRAY));
                    for cat in &["전체", "해연", "잔영"] {
                        if ui
                            .selectable_label(self.craft_grade_filter == *cat, *cat)
                            .clicked()
                        {
                            self.craft_grade_filter = cat.to_string();
                            self.select_first_filtered_recipe();
                        }
                    }
                });

                ui.add_space(2.0);

                // 3. 부위 3분할 필터 ([전체], [무기], [방어구], [장신구])
                ui.horizontal(|ui| {
                    ui.label(RichText::new("부위:").size(11.0).color(Color32::GRAY));
                    if ui
                        .selectable_label(self.craft_slot_filter.is_none(), "전체")
                        .clicked()
                    {
                        self.craft_slot_filter = None;
                        self.select_first_filtered_recipe();
                    }
                    if ui
                        .selectable_label(
                            self.craft_slot_filter == Some(EquipmentSlot::Weapon),
                            "무기",
                        )
                        .clicked()
                    {
                        self.craft_slot_filter = Some(EquipmentSlot::Weapon);
                        self.select_first_filtered_recipe();
                    }
                    if ui
                        .selectable_label(
                            self.craft_slot_filter == Some(EquipmentSlot::Armor),
                            "방어구",
                        )
                        .clicked()
                    {
                        self.craft_slot_filter = Some(EquipmentSlot::Armor);
                        self.select_first_filtered_recipe();
                    }
                    if ui
                        .selectable_label(
                            self.craft_slot_filter == Some(EquipmentSlot::Accessory),
                            "장신구",
                        )
                        .clicked()
                    {
                        self.craft_slot_filter = Some(EquipmentSlot::Accessory);
                        self.select_first_filtered_recipe();
                    }
                });

                ui.add_space(4.0);

                // 4. 검색창
                ui.horizontal(|ui| {
                    ui.label("검색:");
                    ui.text_edit_singleline(&mut self.craft_search_query);
                });

                ui.add_space(4.0);

                // 5. 필터링된 레시피 목록 계산 (직업별 1:1 무기 + 방어구 + 장신구 매핑 적용)
                let query = self.craft_search_query.trim().to_lowercase();
                let job_filter = self.job_only_filter;
                let current_job = self.job_name.clone();
                let grade_filter = self.craft_grade_filter.clone();
                let slot_filter = self.craft_slot_filter;

                let filtered_recipes: Vec<RecipeRecord> = self
                    .all_recipes
                    .iter()
                    .filter(|r| {
                        if job_filter && !is_job_equipment(&current_job, &r.display_name) {
                            return false;
                        }
                        if grade_filter != "전체" && !r.display_name.contains(&grade_filter) {
                            return false;
                        }
                        if let Some(target_slot) = slot_filter {
                            if get_equipment_slot(&r.display_name) != target_slot {
                                return false;
                            }
                        }
                        if !query.is_empty() && !r.display_name.to_lowercase().contains(&query) {
                            return false;
                        }
                        true
                    })
                    .cloned()
                    .collect();

                ui.label(
                    RichText::new(format!("제작 가능 장비 ({}종)", filtered_recipes.len()))
                        .size(12.0)
                        .color(Color32::LIGHT_BLUE),
                );

                let left_recipe_height = (total_height - 180.0).max(450.0);
                egui::ScrollArea::vertical()
                    .id_salt("craft_recipes_list_scroll")
                    .drag_to_scroll(false)
                    .auto_shrink([false, false])
                    .min_scrolled_height(left_recipe_height)
                    .max_height(left_recipe_height)
                    .show(ui, |ui| {
                        for rec in filtered_recipes {
                            let is_sel = match &self.selected_recipe {
                                Some(cur) => cur.display_name == rec.display_name,
                                None => false,
                            };

                            let resp = ui.selectable_label(
                                is_sel,
                                RichText::new(&rec.display_name).strong(),
                            );
                            if resp.clicked() {
                                self.selected_recipe = Some(rec);
                                self.recalculate_cost();
                            }
                        }
                    });
            });

            ui.separator();

            // 우측 메인 패널
            ui.vertical(|ui| {
                if let Some(recipe) = self.selected_recipe.clone() {
                    let norm_succ = if recipe.success_rate > 0.0 && recipe.success_rate <= 1.0 {
                        recipe.success_rate * 100.0
                    } else {
                        recipe.success_rate
                    };
                    let norm_crit = if recipe.critical_success_rate > 0.0 && recipe.critical_success_rate <= 1.0 {
                        recipe.critical_success_rate * 100.0
                    } else {
                        recipe.critical_success_rate
                    };

                    ui.horizontal(|ui| {
                        ui.heading(
                            RichText::new(&recipe.display_name)
                                .size(20.0)
                                .color(Color32::WHITE),
                        );

                        // 사용자 요청 버그 수정: 10000% 또는 9000%, 1200% 표기 문제 완전 정규화
                        ui.label(
                            RichText::new(format!(
                                "(성공률: {}% / 대성공: {}%)",
                                norm_succ as i32,
                                norm_crit as i32
                            ))
                            .color(Color32::GRAY),
                        );

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // 대성공 감안 체크박스 (기본 OFF)
                            if ui
                                .checkbox(
                                    &mut self.craft_consider_critical,
                                    RichText::new("대성공 감안 (+1개)").size(12.0),
                                )
                                .on_hover_text("대성공 시 완제품 2개 산출 기대값을 반영하여 소요 시도 횟수를 절감합니다.")
                                .changed()
                            {
                                self.recalculate_cost();
                            }

                            ui.add_space(8.0);

                            // 성공률 감안 체크박스 (기본 ON)
                            if ui
                                .checkbox(
                                    &mut self.craft_consider_success,
                                    RichText::new("성공률 감안").size(12.0).strong(),
                                )
                                .on_hover_text("성공률 미달 시 추가 시도 비용(1/성공률)을 제작비 기대값에 반영합니다.")
                                .changed()
                            {
                                self.recalculate_cost();
                            }

                            ui.add_space(12.0);

                            if ui.button(" + ").clicked() {
                                self.craft_quantity = (self.craft_quantity + 1).min(99);
                                self.recalculate_cost();
                            }
                            ui.label(
                                RichText::new(format!("{} 개", self.craft_quantity))
                                    .size(15.0)
                                    .strong(),
                            );
                            if ui.button(" - ").clicked() {
                                self.craft_quantity = (self.craft_quantity - 1).max(1);
                                self.recalculate_cost();
                            }
                            ui.label("제작 수량:");
                        });
                    });

                    ui.add_space(8.0);

                    // 3단 가격 비교 카드 그리드
                    if let Some(calc) = &self.cost_calculation {
                        let finished_str = if let Some(fp) = calc.product_total_market_price {
                            format!("{} 데카", format_number(fp))
                        } else {
                            "매물 없음".to_string()
                        };

                        let full_craft_str = format!("{} 데카", format_number(calc.total_material_cost));
                        let actual_craft_str = format!("{} 데카", format_number(calc.user_inventory_gap_cost));

                        let b_sub_text = if calc.consider_success && calc.consider_critical {
                            format!("성공률 {}% + 대성공 {}% 감안 (평균 {:.2}회 시도)", norm_succ as i32, norm_crit as i32, calc.expected_attempts)
                        } else if calc.consider_success {
                            format!("성공률 {}% 감안 (평균 {:.2}회 시도)", norm_succ as i32, calc.expected_attempts)
                        } else if calc.consider_critical {
                            format!("대성공 {}% 감안 (평균 {:.2}회 시도)", norm_crit as i32, calc.expected_attempts)
                        } else {
                            "100% 성공 기준 (기본 1회 원가)".to_string()
                        };

                        let c_sub_text = if calc.consider_success || calc.consider_critical {
                            "가방+내창고+공용창고 차감 + 기대값 반영"
                        } else {
                            "가방+내창고+공용창고 차감 후"
                        };

                        ui.columns(3, |cols| {
                            cols[0].group(|ui| {
                                ui.label(RichText::new("A. 완제품 마켓 최저가").size(12.0).color(Color32::GRAY));
                                ui.label(
                                    RichText::new(finished_str)
                                        .size(18.0)
                                        .strong()
                                        .color(Color32::LIGHT_BLUE),
                                );
                                ui.label(RichText::new("마켓 완제품 즉시 구매 시").size(11.0).color(Color32::DARK_GRAY));
                            });

                            cols[1].group(|ui| {
                                ui.label(RichText::new("B. 재료 전량 구매 제작비").size(12.0).color(Color32::GRAY));
                                ui.label(
                                    RichText::new(full_craft_str)
                                        .size(18.0)
                                        .strong()
                                        .color(Color32::from_rgb(250, 190, 60)),
                                );
                                ui.label(RichText::new(b_sub_text).size(11.0).color(Color32::DARK_GRAY));
                            });

                            cols[2].group(|ui| {
                                ui.label(RichText::new("C. 보유 재고 차감 실구매액").size(12.0).color(Color32::GRAY));
                                ui.label(
                                    RichText::new(actual_craft_str)
                                        .size(18.0)
                                        .strong()
                                        .color(Color32::from_rgb(110, 230, 140)),
                                );
                                ui.label(RichText::new(c_sub_text).size(11.0).color(Color32::DARK_GRAY));
                            });
                        });

                        ui.add_space(8.0);

                        // 추천 판정 배너 (네모 박스 깨짐 방지: [추천], [주의] 기호 사용)
                        let verdict_text = &calc.recommendation;
                        let is_craft_recommended = calc.saving_amount.unwrap_or(0) > 0;
                        let banner_bg = if is_craft_recommended {
                            Color32::from_rgb(25, 55, 35)
                        } else {
                            Color32::from_rgb(55, 30, 30)
                        };

                        egui::Frame::none()
                            .fill(banner_bg)
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(80, 120, 90)))
                            .inner_margin(egui::Margin::symmetric(14.0, 8.0))
                            .rounding(6.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let tag = if is_craft_recommended { "[추천]" } else { "[주의]" };
                                    ui.label(
                                        RichText::new(tag)
                                            .size(14.0)
                                            .strong()
                                            .color(if is_craft_recommended { Color32::GREEN } else { Color32::YELLOW }),
                                    );
                                    ui.label(
                                        RichText::new(verdict_text)
                                            .size(14.0)
                                            .strong()
                                            .color(Color32::WHITE),
                                    );
                                });
                            });

                        ui.add_space(10.0);
                        ui.separator();

                        // 필수 재료 상세 명세 테이블
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("필수 재료 최저가 및 소요비용 산출 명세")
                                    .size(14.0)
                                    .strong(),
                            );
                            ui.label(
                                RichText::new("(보유량: 내가방 + 내창고 + 공용창고 실시간 합산)")
                                    .size(11.0)
                                    .color(Color32::from_rgb(100, 200, 255)),
                            );
                        });

                        let table_height = (total_height - 250.0).max(350.0);
                        egui::ScrollArea::vertical()
                            .id_salt("craft_materials_table_scroll")
                            .drag_to_scroll(false)
                            .auto_shrink([false, false])
                            .min_scrolled_height(table_height)
                            .max_height(table_height)
                            .show(ui, |ui| {
                                egui::Grid::new("craft_ingredients_grid")
                                    .striped(true)
                                    .min_col_width(90.0)
                                    .show(ui, |ui| {
                                        ui.label(RichText::new("재료명").strong().color(Color32::GRAY));
                                        ui.label(RichText::new("필요 수량").strong().color(Color32::GRAY));
                                        ui.label(RichText::new("총 보유량 (가방+창고)").strong().color(Color32::GRAY));
                                        ui.label(RichText::new("부족 수량").strong().color(Color32::GRAY));
                                        ui.label(RichText::new("최저가 (단가)").strong().color(Color32::GRAY));
                                        ui.label(RichText::new("총 소요비용").strong().color(Color32::GRAY));
                                        ui.end_row();

                                        for ing in &calc.ingredients {
                                            let unit_price_str = if let Some(p) = ing.unit_min_price {
                                                format!("{} 데카", format_number(p))
                                            } else {
                                                "시세없음".to_string()
                                            };

                                            let cost_str = if let Some(c) = ing.gap_cost {
                                                format!("{} 데카", format_number(c))
                                            } else {
                                                "-".to_string()
                                            };

                                            ui.label(&ing.display_name);
                                            let req_str = if calc.consider_success || calc.consider_critical {
                                                format!("{}개 (기대: {}개)", ing.total_required, ing.expected_required)
                                            } else {
                                                format!("{}개", ing.total_required)
                                            };
                                            ui.label(req_str);

                                            let owned_label = ui.label(
                                                RichText::new(format!("{} 개", ing.owned_count))
                                                    .color(Color32::from_rgb(130, 200, 255))
                                                    .strong(),
                                            );
                                            owned_label.on_hover_text(format!(
                                                "내가방: {}개\n내창고: {}개\n공용창고: {}개",
                                                ing.bag_count, ing.char_storage_count, ing.account_storage_count
                                            ));

                                            let shortage_str = format!("{}개", ing.shortage_count);
                                            ui.label(
                                                RichText::new(shortage_str)
                                                    .color(if ing.shortage_count > 0 {
                                                        Color32::LIGHT_RED
                                                    } else {
                                                        Color32::GREEN
                                                    }),
                                            );
                                            ui.label(unit_price_str);
                                            ui.label(
                                                RichText::new(cost_str)
                                                    .strong()
                                                    .color(Color32::from_rgb(255, 200, 80)),
                                            );
                                            ui.end_row();
                                        }
                                    });
                            });
                    }
                } else {
                    ui.label("제작 레시피를 선택해 주세요.");
                }
            });
        });
    }

    /// 필터 조건에 맞는 첫 번째 레시피 선택
    fn select_first_filtered_recipe(&mut self) {
        let job_filter = self.job_only_filter;
        let current_job = self.job_name.clone();
        let grade_filter = self.craft_grade_filter.clone();
        let slot_filter = self.craft_slot_filter;

        if let Some(rec) = self.all_recipes.iter().find(|r| {
            if job_filter && !is_job_equipment(&current_job, &r.display_name) {
                return false;
            }
            if grade_filter != "전체" && !r.display_name.contains(&grade_filter) {
                return false;
            }
            if let Some(target_slot) = slot_filter {
                if get_equipment_slot(&r.display_name) != target_slot {
                    return false;
                }
            }
            true
        }) {
            self.selected_recipe = Some(rec.clone());
            self.recalculate_cost();
        }
    }

    /// [플래너] 내 관심 장비 찜 & 스펙업 총 견적 대시보드 뷰 렌더링
    fn view_timing_analysis(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("specup_planner_scroll")
            .show(ui, |ui| {
                ui.add_space(8.0);

                // 1. 상단 타이틀 배너 (실전 스펙업 플래너 특화)
                egui::Frame::none()
                    .fill(Color32::from_rgb(22, 27, 34))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(48, 54, 61)))
                    .rounding(8.0)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("⭐").size(24.0));
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new("마비노기 모바일 내 관심 장비 찜 & 스펙업 총 견적 플래너")
                                        .size(16.0)
                                        .strong()
                                        .color(Color32::from_rgb(255, 215, 80)),
                                );
                                ui.label(
                                    RichText::new("내가 찜해둔 무기, 방어구, 장신구의 실시간 최저가와 7일 가격 범위를 추적하고, 목표 풀세트를 맞추는 데 필요한 총비용(데카) 견적을 실시간으로 산출합니다.")
                                        .size(12.0)
                                        .color(Color32::from_rgb(180, 190, 205)),
                                );
                            });
                        });
                    });

                ui.add_space(10.0);

                let fav_items = self.get_group_items(ItemGroup::Favorites);

                // 찜한 품목이 없는 경우 (Empty State 안내 박스)
                if fav_items.is_empty() {
                    egui::Frame::none()
                        .fill(Color32::from_rgb(20, 24, 32))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(50, 60, 75)))
                        .rounding(8.0)
                        .inner_margin(24.0)
                        .show(ui, |ui| {
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new("⭐").size(40.0));
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new("아직 찜한 장비나 아이템이 없습니다!")
                                        .size(18.0)
                                        .strong()
                                        .color(Color32::from_rgb(255, 215, 80)),
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new("1번째 [시세] 탭이나 [내 직업 장비] 목록에서 내가 맞추고 싶은 무기, 방어구, 장신구 옆의 '☆' 버튼을 눌러보세요.")
                                        .size(13.0)
                                        .color(Color32::LIGHT_GRAY),
                                );
                                ui.label(
                                    RichText::new("찜한 장비들의 풀세트 스펙업 총 견적과 최저가 바닥권 도달 여부를 한눈에 계산해 드립니다.")
                                        .size(13.0)
                                        .color(Color32::from_rgb(160, 180, 205)),
                                );
                                ui.add_space(14.0);
                                if ui.button(RichText::new("  👉 [시세] 탭으로 이동하여 내 직업 장비 둘러보기  ").size(14.0).strong().color(Color32::WHITE)).clicked() {
                                    self.active_tab = MainTab::MarketPrices;
                                    self.selected_group = ItemGroup::MyJob;
                                    let items = self.get_group_items(ItemGroup::MyJob);
                                    if let Some(first) = items.first() {
                                        self.selected_item = first.clone();
                                    }
                                }
                            });
                        });
                    return;
                }

                // 찜 품목 데이터 모델 및 견적 집계
                struct FavRow {
                    name: String,
                    cur_price: Option<i64>,
                    min_7d: Option<i64>,
                    max_7d: Option<i64>,
                    status_label: &'static str,
                    status_color: Color32,
                    diff_min_str: String,
                    has_recipe: bool,
                }

                let mut rows: Vec<FavRow> = Vec::new();
                let mut total_current_cost: i64 = 0;
                let mut total_optimal_cost: i64 = 0;
                let mut priced_count = 0;

                for name in &fav_items {
                    let cur_price = self.get_cached_price(name);
                    let (min_7d, max_7d) = self.get_period_min_max(name);

                    if let Some(p) = cur_price {
                        total_current_cost += p;
                        priced_count += 1;
                    }
                    if let Some(min_p) = min_7d {
                        total_optimal_cost += min_p;
                    } else if let Some(p) = cur_price {
                        total_optimal_cost += p;
                    }

                    // 시세 상태 판정 (🟢바닥권 / 🔴최고점 / 🟠적정시세)
                    let (status_label, status_color) = if let (Some(cur), Some(min), Some(max)) = (cur_price, min_7d, max_7d) {
                        if cur <= min || cur <= (min as f64 * 1.02) as i64 {
                            ("🟢 바닥권 도달 (득템 찬스!)", Color32::from_rgb(100, 235, 120))
                        } else if cur >= (max as f64 * 0.98) as i64 {
                            ("🔴 최근 최고점 (매수 주의)", Color32::from_rgb(255, 120, 120))
                        } else {
                            ("🟠 적정 시세 횡보", Color32::from_rgb(250, 190, 80))
                        }
                    } else if cur_price.is_some() {
                        ("🟠 데이터 수집 중", Color32::from_rgb(200, 200, 200))
                    } else {
                        ("⚪ 시세 수집 대기", Color32::GRAY)
                    };

                    // 7일 최저가 대비 변동폭
                    let diff_min_str = if let (Some(cur), Some(min)) = (cur_price, min_7d) {
                        if cur <= min {
                            "최저가 동결".to_string()
                        } else {
                            let diff = cur - min;
                            let ratio = if min > 0 { (diff as f64 / min as f64) * 100.0 } else { 0.0 };
                            format!("+{} 데카 (+{:.1}%)", format_number(diff), ratio)
                        }
                    } else {
                        "-".to_string()
                    };

                    // 레시피 존재 여부 (ZZ 양방향 호환)
                    let has_recipe = self.all_recipes.iter().any(|r| {
                        r.display_name == *name
                            || r.display_name == format!("{}ZZ", name)
                            || r.display_name == name.trim_end_matches("ZZ")
                    });

                    rows.push(FavRow {
                        name: name.clone(),
                        cur_price,
                        min_7d,
                        max_7d,
                        status_label,
                        status_color,
                        diff_min_str,
                        has_recipe,
                    });
                }

                // 2. 상단 3대 스펙업 KPI 카드
                let total_fav_count = fav_items.len();
                let saving_potential = (total_current_cost - total_optimal_cost).max(0);
                let saving_ratio = if total_current_cost > 0 {
                    (saving_potential as f64 / total_current_cost as f64) * 100.0
                } else {
                    0.0
                };

                ui.columns(3, |cols| {
                    cols[0].group(|ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("⭐ 찜한 목표 장비 수").size(12.0).color(Color32::from_rgb(255, 215, 80)));
                        ui.label(RichText::new(format!("{} 개 품목", total_fav_count)).size(18.0).strong().color(Color32::WHITE));
                        ui.label(RichText::new(format!("* 실시간 시세 반영: {}/{}개 품목", priced_count, total_fav_count)).size(10.0).color(Color32::GRAY));
                    });
                    cols[1].group(|ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("💰 풀세트 스펙업 총 견적 (현재 최저가)").size(12.0).color(Color32::from_rgb(250, 180, 40)));
                        ui.label(RichText::new(format!("{} 데카", format_number(total_current_cost))).size(18.0).strong().color(Color32::from_rgb(250, 190, 80)));
                        ui.label(RichText::new("현재 마켓 최저가로 풀세트 구매 시 총비용").size(10.0).color(Color32::GRAY));
                    });
                    cols[2].group(|ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("🎯 7일 최저가 기준 최적 견적").size(12.0).color(Color32::from_rgb(100, 235, 120)));
                        ui.label(RichText::new(format!("{} 데카 (-{:.1}%)", format_number(total_optimal_cost), saving_ratio)).size(18.0).strong().color(Color32::from_rgb(100, 235, 120)));
                        ui.label(RichText::new(format!("바닥권 매수 시 약 {} 데카 절약 가능", format_number(saving_potential))).size(10.0).color(Color32::GRAY));
                    });
                });

                ui.add_space(10.0);

                // 3. 찜한 장비 정밀 데이터 테이블
                let mut fav_to_toggle: Option<String> = None;
                let mut recipe_to_jump: Option<String> = None;

                egui::Frame::none()
                    .fill(Color32::from_rgb(20, 24, 32))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(40, 48, 60)))
                    .rounding(8.0)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        egui::Grid::new("specup_fav_grid")
                            .striped(true)
                            .min_col_width(70.0)
                            .spacing([12.0, 7.0])
                            .show(ui, |ui| {
                                // 헤더
                                ui.label(RichText::new("해제").strong().color(Color32::LIGHT_GRAY));
                                ui.label(RichText::new("품목명").strong().color(Color32::LIGHT_GRAY));
                                ui.label(RichText::new("현재 최저가").strong().color(Color32::from_rgb(250, 180, 40)));
                                ui.label(RichText::new("최근 7일 가격 범위").strong().color(Color32::LIGHT_GRAY));
                                ui.label(RichText::new("현재 시세 상태").strong().color(Color32::WHITE));
                                ui.label(RichText::new("7일 최저가 대비 변동").strong().color(Color32::from_rgb(160, 200, 255)));
                                ui.label(RichText::new("제작 계산").strong().color(Color32::from_rgb(255, 215, 0)));
                                ui.end_row();

                                for row in &rows {
                                    // 1. 찜 해제 버튼
                                    if ui.button(RichText::new("★").color(Color32::from_rgb(255, 215, 0)).size(13.0))
                                        .on_hover_text("클릭 시 찜 목록에서 해제")
                                        .clicked()
                                    {
                                        fav_to_toggle = Some(row.name.clone());
                                    }

                                    // 2. 품목명
                                    ui.label(RichText::new(&row.name).strong());

                                    // 3. 현재 최저가
                                    let cur_str = if let Some(p) = row.cur_price {
                                        format!("{} 데카", format_number(p))
                                    } else {
                                        "시세없음".to_string()
                                    };
                                    ui.label(RichText::new(cur_str).color(Color32::from_rgb(250, 180, 40)).strong());

                                    // 4. 최근 7일 가격 범위
                                    let range_str = if let (Some(min), Some(max)) = (row.min_7d, row.max_7d) {
                                        format!("{} ~ {}", format_number(min), format_number(max))
                                    } else {
                                        "-".to_string()
                                    };
                                    ui.label(RichText::new(range_str).color(Color32::from_rgb(160, 175, 195)));

                                    // 5. 시세 상태
                                    ui.label(RichText::new(row.status_label).color(row.status_color).strong());

                                    // 6. 7일 최저가 대비 변동
                                    ui.label(RichText::new(&row.diff_min_str).color(Color32::from_rgb(160, 200, 255)));

                                    // 7. 제작 계산기 바로가기
                                    if row.has_recipe {
                                        if ui.button(RichText::new("🔨 제작비 비교").size(11.0).strong().color(Color32::WHITE))
                                            .on_hover_text("2번째 [제작] 탭으로 이동하여 내 인벤토리 재료 차감 실구매액 계산")
                                            .clicked()
                                        {
                                            recipe_to_jump = Some(row.name.clone());
                                        }
                                    } else {
                                        ui.add_enabled(
                                            false,
                                            egui::Button::new(RichText::new("제작 불가").size(10.0).color(Color32::DARK_GRAY)),
                                        ).on_hover_text("제작 레시피가 없는 드롭/상자 전용 완제품입니다");
                                    }

                                    ui.end_row();
                                }
                            });
                    });

                // 찜 해제 처리
                if let Some(target_fav) = fav_to_toggle {
                    self.toggle_favorite(&target_fav);
                }

                // 제작비 계산기로 이동 처리
                if let Some(jump_name) = recipe_to_jump {
                    let matched_recipe = self.all_recipes.iter().find(|r| {
                        r.display_name == jump_name
                            || r.display_name == format!("{}ZZ", jump_name)
                            || r.display_name == jump_name.trim_end_matches("ZZ")
                    }).cloned();

                    if let Some(rec) = matched_recipe {
                        self.selected_recipe = Some(rec);
                        self.recalculate_cost();
                        self.active_tab = MainTab::CraftCalculator;
                    }
                }

                ui.add_space(16.0);
            });
    }

    /// 탭 4: 가공 제작재료 데카벌이 & 시급 계산기 뷰 렌더링
    fn view_material_profit_calculator(&mut self, ui: &mut egui::Ui) {
        let mut results = MaterialProfitCalculator::calculate(&self.latest_prices, &self.custom_material_durations);

        // 1. 유저 정렬 모드 반영
        match self.material_sort_mode {
            MaterialSortMode::SecondaryProfit => {
                results.sort_by(|a, b| {
                    let pb = b.profit_secondary.or(b.profit_primary).unwrap_or(i64::MIN);
                    let pa = a.profit_secondary.or(a.profit_primary).unwrap_or(i64::MIN);
                    pb.cmp(&pa)
                });
            }
            MaterialSortMode::PrimaryProfit => {
                results.sort_by(|a, b| {
                    let pb = b.profit_primary.unwrap_or(i64::MIN);
                    let pa = a.profit_primary.unwrap_or(i64::MIN);
                    pb.cmp(&pa)
                });
            }
            MaterialSortMode::HourlyProfit => {
                results.sort_by(|a, b| {
                    let hb = b.hourly_secondary.or(b.hourly_primary).unwrap_or(i64::MIN);
                    let ha = a.hourly_secondary.or(a.hourly_primary).unwrap_or(i64::MIN);
                    hb.cmp(&ha)
                });
            }
        }

        // 선택된 품목 기본값 보장 (첫 번째 항목)
        if self.selected_material_name.is_none() || !results.iter().any(|r| Some(&r.name) == self.selected_material_name.as_ref()) {
            if let Some(first) = results.first() {
                self.selected_material_name = Some(first.name.clone());
            }
        }

        egui::ScrollArea::vertical()
            .id_salt("material_profit_scroll")
            .show(ui, |ui| {
                ui.add_space(8.0);

                // 1. 상단 타이틀 배너
                egui::Frame::none()
                    .fill(Color32::from_rgb(22, 27, 34))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(48, 54, 61)))
                    .rounding(8.0)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("💰").size(24.0));
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new("마비노기 모바일 가공 제작재료 데카벌이 플래너 (2-Track 원가 분석)")
                                        .size(16.0)
                                        .strong()
                                        .color(Color32::from_rgb(120, 230, 140)),
                                );
                                ui.label(
                                    RichText::new("거래소 판매 수수료 10% 자동 공제(실수령 90%) 기준! 1차 재료 직구매(최소 시간) vs 2차 재료 자체 가공(노력 투자)의 순익과 수수료 적자 함정을 분석합니다.")
                                        .size(12.0)
                                        .color(Color32::from_rgb(180, 190, 205)),
                                );
                            });
                        });
                    });

                ui.add_space(8.0);

                // 2. 상단 인터랙티브 컨트롤 바 (정렬 토글 + 생산 수량 슬라이더)
                ui.horizontal(|ui| {
                    ui.label(RichText::new("정렬 기준:").strong().color(Color32::LIGHT_GRAY));

                    let sort_options = [
                        (MaterialSortMode::SecondaryProfit, "👑 [루트 B] 자체 가공 순익순"),
                        (MaterialSortMode::PrimaryProfit, "⚡ [루트 A] 1차 직구매 순익순"),
                        (MaterialSortMode::HourlyProfit, "⏱️ 시간당 시급순"),
                    ];
                    for (mode, label) in sort_options {
                        let is_active = self.material_sort_mode == mode;
                        let text = RichText::new(label).color(if is_active {
                            Color32::from_rgb(255, 215, 0)
                        } else {
                            Color32::GRAY
                        }).strong();
                        if ui.selectable_label(is_active, text).clicked() {
                            self.material_sort_mode = mode;
                        }
                    }

                    ui.separator();

                    ui.label(RichText::new("생산 수량:").strong().color(Color32::LIGHT_GRAY));
                    ui.add(egui::Slider::new(&mut self.material_batch_count, 10..=1000).suffix("개"));
                });

                ui.add_space(10.0);

                // 3. 상단 3대 KPI 카드 (노력 vs 순익 경중 평가)
                let top_secondary = results.iter().max_by_key(|r| r.profit_secondary.unwrap_or(i64::MIN));
                let top_primary = results.iter().filter(|r| !r.is_primary_loss).max_by_key(|r| r.profit_primary.unwrap_or(i64::MIN));
                let loss_count = results.iter().filter(|r| r.is_primary_loss).count();

                let batch_count = self.material_batch_count as i64;

                ui.columns(3, |cols| {
                    cols[0].group(|ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("🔥 [루트 B] 자체 가공 최고 수익 (노력 투자 1위)").size(12.0).color(Color32::from_rgb(255, 120, 120)));
                        if let Some(top) = top_secondary {
                            let sec_profit = top.profit_secondary.unwrap_or(0);
                            ui.label(RichText::new(format!("{} (+{} 데카/1회)", top.name, format_number(sec_profit))).size(16.0).strong().color(Color32::from_rgb(255, 90, 90)));
                            let sec_name = top.secondary_ing_name.unwrap_or("-");
                            ui.label(RichText::new(format!("2차 재료 [{}] 자체 가공 시 극대화", sec_name)).size(10.5).color(Color32::GRAY));
                        } else {
                            ui.label(RichText::new("시세 분석 중").size(16.0).color(Color32::GRAY));
                            ui.label(RichText::new("거래소 데이터 대기").size(10.0).color(Color32::DARK_GRAY));
                        }
                    });

                    cols[1].group(|ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("⚡ [루트 A] 1차 직구매 최고 효율 (최소 노력/간편)").size(12.0).color(Color32::from_rgb(120, 220, 255)));
                        if let Some(top_p) = top_primary {
                            let pri_profit = top_p.profit_primary.unwrap_or(0);
                            ui.label(RichText::new(format!("{} (+{} 데카/1회)", top_p.name, format_number(pri_profit))).size(16.0).strong().color(Color32::from_rgb(100, 215, 255)));
                            ui.label(RichText::new(format!("재료 [{}] 즉시 매입 가공 가능", top_p.primary_ing_name)).size(10.5).color(Color32::GRAY));
                        } else {
                            ui.label(RichText::new("직구매 흑자 품목 없음").size(16.0).color(Color32::GRAY));
                            ui.label(RichText::new("모든 완제품 직구매 손해").size(10.0).color(Color32::DARK_GRAY));
                        }
                    });

                    cols[2].group(|ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("⚠️ 직구매 적자 주의 품목").size(12.0).color(Color32::from_rgb(255, 180, 80)));
                        ui.label(RichText::new(format!("{}개 품목", loss_count)).size(16.0).strong().color(Color32::from_rgb(255, 160, 60)));
                        ui.label(RichText::new("1차 재료 직구매 시 수수료로 손해 (자체 가공 필수!)").size(10.5).color(Color32::GRAY));
                    });
                });

                ui.add_space(10.0);

                // 4. 단일 종합 가공 데카벌이 분석 그리드
                let mut newly_selected = None;

                egui::Frame::none()
                    .fill(Color32::from_rgb(20, 24, 32))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(40, 48, 60)))
                    .rounding(8.0)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        egui::Grid::new("material_profit_grid_unified")
                            .striped(true)
                            .min_col_width(70.0)
                            .spacing([12.0, 7.0])
                            .show(ui, |ui| {
                                // 헤더
                                ui.label(RichText::new("품목명 (클릭 상세)").strong().color(Color32::LIGHT_GRAY));
                                ui.label(RichText::new("산출량").strong().color(Color32::LIGHT_GRAY));
                                ui.label(RichText::new("거래소 판매가").strong().color(Color32::from_rgb(250, 180, 40)));
                                ui.label(RichText::new("실수령액 (90%)").strong().color(Color32::from_rgb(180, 230, 160)));
                                ui.label(RichText::new("⚡ [루트 A] 1차 직구매 순익").strong().color(Color32::from_rgb(120, 220, 255)));
                                ui.label(RichText::new("🔥 [루트 B] 2차 자체 가공 순익").strong().color(Color32::from_rgb(255, 120, 120)));
                                ui.label(RichText::new(format!("목표 {}개 자체 순익", batch_count)).strong().color(Color32::WHITE));
                                ui.label(RichText::new("추천 제작 전략").strong().color(Color32::WHITE));
                                ui.end_row();

                                for row in &results {
                                    let is_selected = self.selected_material_name.as_deref() == Some(&row.name);

                                    // 1. 품목명 (클릭 시 선택 토글)
                                    let name_text = if is_selected {
                                        RichText::new(format!("▶ {}", row.name)).color(Color32::from_rgb(255, 215, 0)).strong()
                                    } else {
                                        RichText::new(&row.name).strong()
                                    };
                                    if ui.selectable_label(is_selected, name_text).clicked() {
                                        newly_selected = Some(row.name.clone());
                                    }

                                    // 2. 산출량
                                    ui.label(RichText::new(format!("{}개", row.produced_per_craft)).color(Color32::GRAY));

                                    // 3. 거래소 판매가
                                    let market_str = if let Some(p) = row.market_price {
                                        format!("{} 데카", format_number(p))
                                    } else {
                                        "시세없음".to_string()
                                    };
                                    ui.label(RichText::new(market_str).color(Color32::from_rgb(250, 180, 40)).strong());

                                    // 4. 실수령액 (90%)
                                    let rev_str = if let Some(rev) = row.revenue_net {
                                        format!("{} 데카", format_number(rev))
                                    } else {
                                        "-".to_string()
                                    };
                                    ui.label(RichText::new(rev_str).color(Color32::from_rgb(180, 230, 160)));

                                    // 5. [루트 A] 1차 직구매 순익 (툴팁: 1차 재료비)
                                    let (pri_str, pri_color, pri_tip) = if let Some(net1) = row.profit_primary {
                                        let c_str = format!("{:+}", format_number(net1));
                                        let color = if net1 > 0 {
                                            Color32::from_rgb(100, 235, 120)
                                        } else {
                                            Color32::from_rgb(255, 90, 90)
                                        };
                                        let tip = format!("• 1차 원재료: {} x {}개 (투입비용: {} 데카)\n• 실수령액: {} 데카\n• 순마진: {:+} 데카",
                                            row.primary_ing_name, row.primary_ing_qty, format_number(row.primary_cost.unwrap_or(0)),
                                            format_number(row.revenue_net.unwrap_or(0)), net1);
                                        (c_str, color, tip)
                                    } else {
                                        ("-".to_string(), Color32::GRAY, "시세 미수집".to_string())
                                    };
                                    ui.label(RichText::new(pri_str).strong().color(pri_color))
                                        .on_hover_text(pri_tip);

                                    // 6. [루트 B] 2차 자체 가공 순익 (툴팁: 2차 재료비 및 가공 단계)
                                    let (sec_str, sec_color, sec_tip) = if let Some(net2) = row.profit_secondary {
                                        let c_str = format!("{:+}", format_number(net2));
                                        let color = if net2 >= 70 {
                                            Color32::from_rgb(255, 215, 0)
                                        } else if net2 > 0 {
                                            Color32::from_rgb(100, 235, 120)
                                        } else {
                                            Color32::from_rgb(255, 90, 90)
                                        };
                                        let sec_name = row.secondary_ing_name.unwrap_or("-");
                                        let tip = format!("• 2차 하위재료: [{}] 자체 가공 단계 거침\n• 환산 투입원가: {} 데카\n• 2회 연속 가공 순마진: {:+} 데카",
                                            sec_name, format_number(row.secondary_cost.unwrap_or(0)), net2);
                                        (c_str, color, tip)
                                    } else {
                                        ("- (단일 단계)".to_string(), Color32::GRAY, "2차 가공 단계 없음 (단일 단계 가공품)".to_string())
                                    };
                                    ui.label(RichText::new(sec_str).strong().color(sec_color))
                                        .on_hover_text(sec_tip);

                                    // 7. 목표 N개 자체 순익
                                    let best_profit = row.profit_secondary.or(row.profit_primary);
                                    if let Some(p) = best_profit {
                                        let total_batch_net = (p as f64 * (batch_count as f64 / row.produced_per_craft.max(1) as f64)).round() as i64;
                                        let b_color = if total_batch_net > 0 { Color32::WHITE } else { Color32::LIGHT_RED };
                                        ui.label(RichText::new(format!("{} 데카", format_number(total_batch_net))).color(b_color));
                                    } else {
                                        ui.label("-");
                                    }

                                    // 8. 추천 제작 전략 뱃지
                                    let badge_color = match row.strategy_code {
                                        3 => Color32::from_rgb(255, 215, 0),
                                        2 => Color32::from_rgb(100, 235, 120),
                                        1 => Color32::from_rgb(255, 90, 90),
                                        _ => Color32::GRAY,
                                    };
                                    ui.label(RichText::new(row.strategy_label).color(badge_color).strong());

                                    ui.end_row();
                                }
                            });
                    });

                if let Some(sel) = newly_selected {
                    self.selected_material_name = Some(sel);
                }

                ui.add_space(14.0);

                // 5. 하단 원가 상세 분해 & 스마트 가이드 패널 (선택된 품목)
                if let Some(selected_name) = &self.selected_material_name {
                    if let Some(sel_row) = results.iter().find(|r| &r.name == selected_name) {
                        egui::Frame::none()
                            .fill(Color32::from_rgb(24, 28, 36))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(60, 70, 85)))
                            .rounding(8.0)
                            .inner_margin(14.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("🔍").size(20.0));
                                    ui.label(RichText::new(format!("[{}] 1회 생산 원가 상세 분해 & 스마트 추천 가이드", sel_row.name)).size(15.0).strong().color(Color32::from_rgb(255, 215, 100)));
                                    ui.label(RichText::new(format!("(1회 {}개 산출 | 완제품 판매가: {} 데카 | 실수령: {} 데카)",
                                        sel_row.produced_per_craft,
                                        sel_row.market_price.map_or("-".to_string(), format_number),
                                        sel_row.revenue_net.map_or("-".to_string(), format_number)
                                    )).size(12.0).color(Color32::GRAY));
                                });

                                ui.add_space(8.0);
                                ui.separator();
                                ui.add_space(8.0);

                                ui.columns(2, |cols| {
                                    // 좌측: 루트 A (1차 직구매)
                                    cols[0].group(|ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new("⚡ [루트 A] 1차 원재료 직구매 제작").strong().color(Color32::from_rgb(120, 220, 255)));
                                            ui.label(RichText::new("(최소 시간/간편 제작)").size(11.0).color(Color32::GRAY));
                                        });
                                        ui.add_space(4.0);

                                        let p_cost = sel_row.primary_cost.map_or("-".to_string(), format_number);
                                        let p_net = sel_row.profit_primary.map_or("-".to_string(), |n| format!("{:+}", format_number(n)));
                                        let p_hourly = sel_row.hourly_primary.map_or("-".to_string(), |h| format!("{:+}", format_number(h)));
                                        let color = if sel_row.profit_primary.unwrap_or(0) > 0 { Color32::from_rgb(100, 235, 120) } else { Color32::from_rgb(255, 90, 90) };

                                        ui.label(format!("• 투입 원재료: {} x {}개", sel_row.primary_ing_name, sel_row.primary_ing_qty));
                                        ui.label(format!("• 1차 직구매 총비용: {} 데카", p_cost));
                                        ui.label(RichText::new(format!("• 1회 순마진: {} 데카", p_net)).strong().color(color));
                                        ui.label(RichText::new(format!("• 시간당 시급: {} 데카/h (소요시간: {}초)", p_hourly, sel_row.duration_secs)).color(Color32::LIGHT_GRAY));
                                    });

                                    // 우측: 루트 B (2차 자체 가공)
                                    cols[1].group(|ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new("🔥 [루트 B] 2차 원재료 자체 가공 제작").strong().color(Color32::from_rgb(255, 120, 120)));
                                            ui.label(RichText::new("(노력/시간 2배 투자)").size(11.0).color(Color32::GRAY));
                                        });
                                        ui.add_space(4.0);

                                        if let Some(sec_name) = sel_row.secondary_ing_name {
                                            let s_cost = sel_row.secondary_cost.map_or("-".to_string(), format_number);
                                            let s_net = sel_row.profit_secondary.map_or("-".to_string(), |n| format!("{:+}", format_number(n)));
                                            let s_hourly = sel_row.hourly_secondary.map_or("-".to_string(), |h| format!("{:+}", format_number(h)));
                                            let s_color = if sel_row.profit_secondary.unwrap_or(0) >= 70 { Color32::from_rgb(255, 215, 0) } else { Color32::from_rgb(100, 235, 120) };

                                            ui.label(format!("• 2차 하위재료: [{}] 자체 가공 단계 거침", sec_name));
                                            ui.label(format!("• 환산 투입원가: {} 데카", s_cost));
                                            ui.label(RichText::new(format!("• 2회 연속 순마진: {} 데카", s_net)).strong().color(s_color));
                                            ui.label(RichText::new(format!("• 시간당 시급: {} 데카/h (소요시간: {}초)", s_hourly, sel_row.duration_secs * 2)).color(Color32::LIGHT_GRAY));
                                        } else {
                                            ui.label("• 2차 하위 가공 단계가 없는 단일 단계 가공품입니다.");
                                            ui.label("• 1차 원재료를 직구매하여 즉시 가공하는 [루트 A]를 이용하십시오.");
                                            ui.label(RichText::new("• 단일 가공만으로도 안정적인 마진 확보 가능").color(Color32::from_rgb(100, 235, 120)));
                                        }
                                    });
                                });

                                ui.add_space(8.0);

                                // 하단 스마트 인사이트 배너
                                egui::Frame::none()
                                    .fill(Color32::from_rgb(16, 20, 26))
                                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(45, 55, 70)))
                                    .rounding(6.0)
                                    .inner_margin(10.0)
                                    .show(ui, |ui| {
                                        if sel_row.is_primary_loss {
                                            let saved = sel_row.profit_secondary.unwrap_or(0) - sel_row.profit_primary.unwrap_or(0);
                                            ui.label(RichText::new(format!("⚠️ [직구매 적자 경고]: [{}]은 1차 재료를 거래소에서 바로 사서 제작하면 10% 판매 수수료 때문에 1회당 {} 데카의 손해가 발생합니다! 반드시 2차 재료 [{}]부터 자체 가공하십시오. (1회당 +{} 데카 흑자 반전)",
                                                sel_row.name,
                                                sel_row.profit_primary.map_or(0, |n| -n),
                                                sel_row.secondary_ing_name.unwrap_or("하위재료"),
                                                saved
                                            )).color(Color32::from_rgb(255, 100, 100)).strong());
                                        } else if let (Some(p1), Some(p2)) = (sel_row.profit_primary, sel_row.profit_secondary) {
                                            let saved = p2 - p1;
                                            ui.label(RichText::new(format!("💡 [노력 대비 추천]: 2차 재료 [{}]부터 직접 가공하면 1회당 {} 데카가 더 절약되어 순이익이 크게 증가합니다! 시간 여유가 있을 때는 자체 가공을, 빠른 회전이 필요할 때는 직구매를 추천합니다.",
                                                sel_row.secondary_ing_name.unwrap_or("하위재료"),
                                                saved
                                            )).color(Color32::from_rgb(120, 230, 140)).strong());
                                        } else {
                                            ui.label(RichText::new("💡 [간편 제작 추천]: 1차 재료를 거래소에서 바로 매입하여 가공해도 안정적인 수익이 보장되는 품목입니다.").color(Color32::from_rgb(100, 215, 255)).strong());
                                        }
                                    });
                            });
                    }
                }

                ui.add_space(16.0);
            });
    }
}

/// 숫자 1000단위 콤마 포맷터
fn format_number(val: i64) -> String {
    let s = val.to_string();
    let is_negative = s.starts_with('-');
    let raw = if is_negative { &s[1..] } else { &s };

    let mut result = String::new();
    let len = raw.len();
    for (i, c) in raw.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }

    if is_negative {
        format!("-{}", result)
    } else {
        result
    }
}
