# MobiMarket Antigravity Plugin & MCP Server

MobiMarket 프로젝트의 SQLite 시세 데이터베이스(`mobi_market.db`)를 바탕으로, Antigravity 에이전트가 언제든 마비노기 모바일 거래소 시세를 확인하고 제작 마진을 계산할 수 있도록 지원하는 MCP 도구 플러그인입니다.

## 제공되는 MCP 도구

1. `mobi_market_get_price(item_name: string)`
   - 특정 아이템의 현재 최저가, 기간 최저점, 기간 최고점, 기간 평균가, 재고 상태, 최근 수집 시각 반환.
   - `ZZ` 접미사 자동 양방향 호환 지원.

2. `mobi_market_list_monitored()`
   - 현재 거래소에서 수집/모니터링 중인 전체 품목의 최신 시세 리스트 반환.

3. `mobi_market_calculate_craft(recipe_name: string, success_override?: number)`
   - 특정 완제품 제작 시 소요되는 실시간 재료비 총합, 성공률(기본 50%), 대성공 보너스(기본 12%), 거래소 판매 수수료(10%)를 종합 반영한 1회당 기대 순이익 및 ROI(수익률) 계산.

## 사용 방법

- **Antigravity 연동**: `plugin.json`이 포함된 `plugins/mobimarket/` 디렉토리를 Antigravity Customization 플러그인으로 등록합니다.
- **독립 실행 테스트**:
  ```bash
  python plugins/mobimarket/server.py
  ```
