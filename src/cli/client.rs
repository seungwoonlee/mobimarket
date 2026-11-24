use crate::cli::models::*;
use crate::config::CLI_PATH;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

const CLI_TIMEOUT: Duration = Duration::from_secs(30);

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// 마비노기 모바일 게임 프로세스(MabinogiMobile.exe) 실행 여부 확인 (창 팝업 없음)
pub fn is_game_process_running() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let output = std::process::Command::new("tasklist")
            .creation_flags(CREATE_NO_WINDOW)
            .args(["/FI", "IMAGENAME eq MabinogiMobile.exe", "/NH"])
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

#[derive(Default)]
pub struct CliClient {
    cli_path: String,
}

impl CliClient {
    pub fn new() -> Self {
        Self {
            cli_path: CLI_PATH.to_string(),
        }
    }

    /// 콘솔 창 없이 백그라운드에서 실행되며 타임아웃 시 종료되는 Command 생성
    fn build_cmd(&self) -> Command {
        let mut cmd = Command::new(&self.cli_path);
        cmd.kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }

    /// 공용 CLI 실행 헬퍼 (타임아웃 + 종료 코드 + stderr 검사)
    async fn run_cli_stdout(&self, args: &[&str], op_name: &str) -> Result<String, String> {
        let mut cmd = self.build_cmd();
        cmd.args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = tokio::time::timeout(CLI_TIMEOUT, cmd.output())
            .await
            .map_err(|_| format!("{} CLI 응답 시간 초과({}초)", op_name, CLI_TIMEOUT.as_secs()))?
            .map_err(|e| format!("{} CLI 실행 실패: {}", op_name, e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "{} 에러 종료({:?}): {}",
                op_name,
                output.status.code(),
                stderr.trim()
            ));
        }

        Ok(stdout)
    }

    /// 게임 CLI status 확인 (pipe connected 여부)
    pub async fn check_status(&self) -> Result<CliStatus, String> {
        let stdout = self.run_cli_stdout(&["status"], "status").await?;
        serde_json::from_str::<CliStatus>(&stdout)
            .map_err(|e| format!("status JSON 파싱 실패: {} (출력: {})", e, stdout))
    }

    /// 내 캐릭터 정보 및 현재 직업 확인 (get_my_info)
    pub async fn get_my_info(&self) -> Result<MyInfoResponse, String> {
        let stdout = self.run_cli_stdout(&["get_my_info"], "get_my_info").await?;
        serde_json::from_str::<MyInfoResponse>(&stdout)
            .map_err(|e| format!("get_my_info JSON 파싱 실패: {} (출력: {})", e, stdout))
    }

    /// 마켓 시세 일괄 조회 (names 배열)
    pub async fn get_market_prices(&self, names: &[String]) -> Result<MarketPricesResponse, String> {
        let body = serde_json::json!({ "names": names }).to_string();
        let stdout = self
            .run_cli_stdout(&["get_market_prices", &body], "get_market_prices")
            .await?;
        serde_json::from_str::<MarketPricesResponse>(&stdout)
            .map_err(|e| format!("get_market_prices JSON 파싱 실패: {} (출력: {})", e, stdout))
    }

    /// 전체 제작 레시피 조회 (get_craftable_items)
    pub async fn get_craftable_items(&self) -> Result<CraftableItemsResponse, String> {
        let stdout = self
            .run_cli_stdout(&["get_craftable_items"], "get_craftable_items")
            .await?;
        serde_json::from_str::<CraftableItemsResponse>(&stdout).map_err(|e| {
            format!(
                "get_craftable_items JSON 파싱 실패: {} (출력 길이: {})",
                e,
                stdout.len()
            )
        })
    }

    /// 가방 + 내창고 + 공용창고 전체 보유 아이템 조회 (get_items)
    pub async fn get_items(&self) -> Result<Vec<OwnedItem>, String> {
        let stdout = self.run_cli_stdout(&["get_items"], "get_items").await?;
        serde_json::from_str::<Vec<OwnedItem>>(&stdout)
            .map_err(|e| format!("get_items JSON 파싱 실패: {} (출력 길이: {})", e, stdout.len()))
    }
}
