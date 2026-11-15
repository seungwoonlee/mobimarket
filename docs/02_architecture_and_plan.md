# MobiMarket 상세 개발 계획 및 아키텍처 설계서

## 1. 시스템 아키텍처 개요

MobiMarket은 Rust 기반의 고성능 단일 실행 파일(`.exe`)로 빌드되며, 내장 SQLite 데이터베이스, 공식 CLI 연동 파이프라인, 내장 임베디드 웹 서버(Axum + rust-embed), 그리고 반응형 웹 UI로 구성됩니다.

```
+-----------------------------------------------------------------------------------------+
|                                    MobiMarket.exe                                       |
|                                                                                         |
|  +--------------------+    +--------------------+    +-------------------------------+  |
|  |   Frontend (Web)   |    |    Axum Server     |    |        CLI Controller         |  |
|  | - React + TS + Vite|    | - REST API / SSE   |    | - Named Pipe / Process Exec   |  |
|  | - Tailwind + Charts|    | - Static Serve     |    | - Rate Limiter (1s Detail)    |  |
|  | (rust-embed 내장)  |<-->|   (Embedded Asset) |<-->| - 5s Snapshot Cache Guard     |  |
|  +--------------------+    +---------+----------+    +---------------+---------------+  |
|                                      |                               |                  |
|                                      v                               v                  |
|                            +--------------------+        +-----------------------+      |
|                            |  SQLite Store      |        | MabinogiMobile_CLI.exe|      |
|                            | - market_history   |        | (공식 게임 중계 파이프) |      |
|                            | - craft_recipes    |        +-----------------------+      |
|                            | - user_watchlist   |                                       |
|                            +--------------------+                                       |
+-----------------------------------------------------------------------------------------+
```

---

## 2. 세부 기술 스택 및 라이브러리 선정

### 2.1 Backend (Rust)
- **런타임 / 비동기 I/O**: `tokio` (multi-thread)
- **웹 서버 / API**: `axum` (고성능, 비동기, 정적 파일 핸들링)
- **정적 파일 임베딩**: `rust-embed` (프론트엔드 빌드 결과물을 `.exe` 바이너리 내부에 패킹)
- **데이터베이스**: `rusqlite` (bundled feature 활성화로 추가 C 라이브러리 설치 없는 완전한 정적 링크)
- **직렬화 / 역직렬화**: `serde`, `serde_json`
- **CLI 프로세스 제어**: `tokio::process::Command` (비동기 서브프로세스 실행 및 stdout 파이프 파싱)
- **브라우저 자동 실행**: `open` (앱 기동 시 브라우저 자동 오픈: `http://127.0.0.1:4829`)
- **시간 처리**: `chrono` (KST 기준 타임스탬프 처리)

### 2.2 Frontend (Embedded Web UI)
- **프레임워크**: React 18 / 19 + TypeScript + Vite
- **스타일링**: TailwindCSS (다크 테마 기본, FHD 1920x1080 해상도 최적화)
- **차트 엔진**: `Chart.js` + `react-chartjs-2` 또는 `TradingView Lightweight Charts` (시계열 캔들/라인 차트 및 줌/패닝 지원)
- **아이콘**: `lucide-react`
- **상태 관리 & 데이터 페칭**: `TanStack Query (React Query)` + `axios`

---

## 3. 데이터베이스 스키마 설계 (`market_history.db`)

단일 SQLite 파일은 `%APPDATA%\MobiMarket\market_history.db`에 자동 생성 및 보관됩니다.

```sql
-- 1. 마켓 시세 스냅샷 테이블
CREATE TABLE IF NOT EXISTS market_snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_name TEXT NOT NULL,
    rarity TEXT,
    star INTEGER,
    season TEXT,
    min_unit_price INTEGER NOT NULL,
    in_stock INTEGER NOT NULL, -- 0: 품절, 1: 재고있음
    scraped_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_snapshots_search ON market_snapshots(item_name, rarity, star, season, scraped_at);
CREATE INDEX IF NOT EXISTS idx_snapshots_item_time ON market_snapshots(item_name, scraped_at);

-- 2. 상세 호가 및 평균 거래가 히스토리
CREATE TABLE IF NOT EXISTS price_details (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_name TEXT NOT NULL,
    rarity TEXT,
    star INTEGER,
    season TEXT,
    min_ask_price INTEGER,
    max_ask_price INTEGER,
    average_unit_price INTEGER,
    tier1_price INTEGER,
    tier1_count INTEGER,
    tier2_price INTEGER,
    tier2_count INTEGER,
    tier3_price INTEGER,
    tier3_count INTEGER,
    scraped_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_details_item_time ON price_details(item_name, scraped_at);

-- 3. 제작 및 가공 레시피 테이블
CREATE TABLE IF NOT EXISTS recipes (
    id TEXT PRIMARY KEY, -- e.g. "craft_못", "alter_철괴(철 광석)"
    recipe_type TEXT NOT NULL, -- 'craft' | 'alter'
    display_name TEXT NOT NULL,
    category TEXT,
    produced_quantity INTEGER NOT NULL DEFAULT 1,
    success_rate REAL DEFAULT 100.0,
    critical_success_rate REAL DEFAULT 0.0,
    ingredients_json TEXT NOT NULL, -- [{"name":"철괴", "required":2}]
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- 4. 사용자 관심 품목(Watchlist)
CREATE TABLE IF NOT EXISTS watchlist (
    item_name TEXT PRIMARY KEY,
    category TEXT,
    target_buy_price INTEGER, -- 목표 구매가 알림용
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

---

## 4. 핵심 파이프라인 및 비즈니스 로직

### 4.1 CLI 연동 및 Rate Limiting
- `C:\Nexon\MabinogiMobile\MabinogiMobile_CLI.exe status`를 5초마다 체크하여 파이프 연결 확인.
- `get_market_prices`:
  - 카테고리별 일괄 수집 또는 관심 품목 일괄 수집 (`{"names": [...]}`).
  - 서버 캐시(5초) 주기를 존중하여 불필요한 고빈도 호출 억제.
- `get_market_price_detail`:
  - 공식 제약사항인 **"초당 1회 (rate_limited, retryAfterSeconds)"** 를 엄격히 준수하기 위해 내부 채널 큐(`tokio::sync::mpsc`)를 두고 최소 1.1초 간격으로 순차 디스패치.

### 4.2 제작 비용 및 수익성 계산 엔진
1. **재료 트리 재귀 전개 (Recursive Crafting Tree)**:
   - 완제품 -> 1차 중간 가공품 -> 기초 원자재 단계까지 DAG(방향성 비순환 그래프)로 모델링.
   - **순환 참조 방어**: DFS 탐색 시 `visited_path: HashSet<RecipeId>`로 루프(A->B->A) 감지 차단 및 최대 깊이 제한(`MAX_CRAFT_DEPTH = 8`).
   - **가공 레시피 시드 데이터(`seed_recipes.json`) 내장**: `get_alterable_items`는 부족한 재료(`MissingIngredients`)만 반환하므로, 이미 보유 중인 재료가 누락되지 않도록 기본 가공 레시피 시드를 내장하여 CLI 실측치와 병합(Merge).
2. **비용 산출 3종 모드**:
   - `Market Buy Cost (완전자재 구매)`: 모든 재료를 경매장 최저가(`MinUnitPrice`)로 즉시 구매 시 드는 비용.
   - `Self Craft Cost (최하위 원자재부터 직접 가공)`: 1차 원자재 가격 + 중간 가공 생산 배수 적용 비용.
   - `Inventory Gap Cost (보유 재고 감안 추가 구매 비용)`: 게임 내 내 인벤토리(`get_items`)를 읽어 부족한 수량만큼만 구매하는 실질 필요 비용.
3. **마진율 및 기대 수익 계산**:
   - $\text{Expected Output} = \text{ProducedPerCraft} \times (\text{SuccessRate}/100) \times (1 + \text{CriticalBonus})$
   - $\text{Net Profit} = (\text{Market Price} \times 0.95) - \text{Unit Craft Cost}$ (수수료 5% 가정, 설정 가능)
   - $\text{ROI (\%)} = (\text{Net Profit} / \text{Unit Craft Cost}) \times 100$

### 4.3 SQLite 동시성 락 방어 및 단일 바이너리 빌드 최적화
- **SQLite 동시성 설정**:
  ```sql
  PRAGMA journal_mode = WAL;
  PRAGMA busy_timeout = 5000;
  PRAGMA synchronous = NORMAL;
  ```
- **Cargo.toml 단일 바이너리 최적화**:
  ```toml
  [profile.release]
  opt-level = "z"        # 크기 최적화
  lto = true             # 링크 타임 최적화
  codegen-units = 1      # 단일 코드젠 유닛으로 LTO 극대화
  panic = "abort"        # 스택 언와인딩 메타데이터 제거
  strip = true           # 디버그 심볼 자동 제거
  ```

---

## 5. 단계별 개발 계획 및 이정표 (Milestones)

- **[Phase 1] 프로젝트 셋업 및 기본 인프라 (현재 단계)**:
  - Cargo 프로젝트 초기화 (`Cargo.toml` 의존성 설정).
  - CLI 연동 모듈 (`src/cli/mod.rs`, `process.rs` - status, capabilities, market commands).
  - SQLite 스토리지 모듈 (`src/store/mod.rs`, migrations).
- **[Phase 2] 백엔드 API & 백그라운드 수집기**:
  - 시세 수집기 및 Rate Limiter (`src/collector/mod.rs`).
  - 레시피 파서 및 다단계 제작 원가 계산 엔진 (`src/calculator/mod.rs`).
  - Axum REST API 핸들러 (`/api/status`, `/api/market/prices`, `/api/market/history`, `/api/craft/recipes`, `/api/watchlist`).
- **[Phase 3] 웹 프론트엔드 UI 개발**:
  - React + Vite + Tailwind 셋업 (`frontend/`).
  - 대시보드 (관심 품목 실시간 시세 카드, 변동률).
  - 가격 변동 그래프 뷰 (차트 줌/기간 필터링/평균가 오버레이).
  - 제작비용 계산기 뷰 (레시피 검색, 재료비 합산, 제작 vs 구매 마진 분석표).
- **[Phase 4] 번들링 및 단일 .exe 패키징**:
  - `rust-embed`로 `frontend/dist` 번들링.
  - 앱 실행 시 브라우저 자동 오픈 로직.
  - 릴리스 빌드 최적화 (LTO, strip, opt-level='z'로 exe 용량 경량화).
- **[Phase 5] 테스트 및 검증**:
  - 단위 테스트 (계산 엔진, CLI 응답 파서).
  - 게임 실제 실행 상태에서의 통합 검증.

---

## 6. 개발 프로세스 준수 규칙
- **서브에이전트 리뷰 원칙**:
  1. 본 계획 수립 후 서브에이전트 리뷰 및 승인.
  2. 구현 각 단계 완료 후 서브에이전트 코드 리뷰 및 승인.
  3. 테스트 및 검증 완료 후 서브에이전트 최종 검증 승인.
