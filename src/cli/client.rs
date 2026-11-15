use crate::cli::models::*;
use crate::config::CLI_PATH;
use std::process::Stdio;
use tokio::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// 마비노기 모바일 게임 프로세스(MabinogiMobile.exe) 실행 여부 확인 (창 팝업 없음)
pub fn is_game_process_running() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let output = std::process::Command::new("tasklist")
            .creation_flags(CREATE_NO_WINDOW)
            .args(&["/FI", "IMAGENAME eq MabinogiMobile.exe", "/NH"])
            .output();
        if let Ok(out) = output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout.contains("MabinogiMobile.exe")
        } else {
            false
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

pub struct CliClient {
    cli_path: String,
}

impl CliClient {
    pub fn new() -> Self {
        Self {
            cli_path: CLI_PATH.to_string(),
        }
    }

    /// 콘솔 창 없이 백그라운드에서 실행되는 Command 생성 (Windows 창 팝업 방지)
    fn build_cmd(&self) -> Command {
        let mut cmd = Command::new(&self.cli_path);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }

    /// 게임 CLI status 확인 (pipe connected 여부)
    pub async fn check_status(&self) -> Result<CliStatus, String> {
        let output = self.build_cmd()
            .arg("status")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| format!("CLI 실행 실패: {}", e))?;

        if !output.status.success() {
            return Err(format!("CLI 에러 종료: {:?}", output.status.code()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str::<CliStatus>(&stdout)
            .map_err(|e| format!("status JSON 파싱 실패: {} (출력: {})", e, stdout))
    }

    /// 내 캐릭터 정보 및 현재 직업 확인 (get_my_info)
    pub async fn get_my_info(&self) -> Result<MyInfoResponse, String> {
        let output = self.build_cmd()
            .arg("get_my_info")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| format!("get_my_info 실행 실패: {}", e))?;

        if !output.status.success() {
            return Err(format!("get_my_info 에러 종료: {:?}", output.status.code()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str::<MyInfoResponse>(&stdout)
            .map_err(|e| format!("get_my_info JSON 파싱 실패: {} (출력: {})", e, stdout))
    }

    /// 마켓 시세 일괄 조회 (names 배열)
    pub async fn get_market_prices(&self, names: &[String]) -> Result<MarketPricesResponse, String> {
        let body = serde_json::json!({ "names": names }).to_string();

        let output = self.build_cmd()
            .arg("get_market_prices")
            .arg(&body)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| format!("get_market_prices CLI 호출 실패: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str::<MarketPricesResponse>(&stdout)
            .map_err(|e| format!("get_market_prices JSON 파싱 실패: {} (출력: {})", e, stdout))
    }

    /// 전체 제작 레시피 조회 (get_craftable_items)
    pub async fn get_craftable_items(&self) -> Result<CraftableItemsResponse, String> {
        let output = self.build_cmd()
            .arg("get_craftable_items")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| format!("get_craftable_items CLI 호출 실패: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str::<CraftableItemsResponse>(&stdout)
            .map_err(|e| format!("get_craftable_items JSON 파싱 실패: {} (출력 길이: {})", e, stdout.len()))
    }

    /// 가방 + 내창고 + 공용창고 전체 보유 아이템 조회 (get_items)
    pub async fn get_items(&self) -> Result<Vec<OwnedItem>, String> {
        let output = self.build_cmd()
            .arg("get_items")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| format!("get_items CLI 호출 실패: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str::<Vec<OwnedItem>>(&stdout)
            .map_err(|e| format!("get_items JSON 파싱 실패: {} (출력 길이: {})", e, stdout.len()))
    }
}
