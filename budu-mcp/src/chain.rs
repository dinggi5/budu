// 클러스터 설정 — 잔액 조회·x402 고르기·익스플로러 링크에 쓰는 값.
//
// src-tauri/src/chain.rs 와 평행한 별도 사본이다(두 크레이트는 의도적으로 분리 — 공유 크레이트를 만들지 않는다).
// 활성 클러스터는 GUI 와 공유하는 settings.json 의 chain_id 로 **런타임 선택**한다(두 프로세스가 자동으로 같은 클러스터).
// 그 선택 판정과 번호·데이터 파일 이름 규칙은 shared/policy.rs 를, 민트·genesis 상수는 shared/solana.rs 를
// GUI 와 **같은 소스로** 컴파일한다 — 사본으로 남는 건 RPC·x402 표기·익스플로러뿐이다.

use crate::policy;
use crate::solana;

/// 한 클러스터의 MCP 측 설정 묶음.
#[derive(Clone, Copy)]
pub struct ChainConfig {
    /// settings.json 의 chain_id 와 매칭 + 데이터 파일 접미사 (policy::SOLANA_*_ID).
    pub chain_id: u64,
    /// settings.json(GUI와 공유)의 rpc_url 이 비어 있을 때의 폴백 공개 RPC.
    pub default_rpc: &'static str,
    /// Circle USDC 민트 (base58 — 대소문자를 가린다).
    pub usdc_mint: &'static str,
    /// USDC 소수 자릿수 (base unit ↔ 십진 변환에 쓴다).
    pub usdc_decimals: u8,
    /// `getGenesisHash` 전체 값 — 지정 RPC 가 이 클러스터인지 대조한다.
    pub genesis_hash: &'static str,
    /// x402 V2 네트워크 표기 (CAIP-2 = `solana:` + genesis 앞 32자). V1 단축명은 쓰지 않는다.
    pub x402_network_caip2: &'static str,
    /// 익스플로러 트랜잭션 URL 접두사·꼬리 — 링크 = 접두사 + tx id + 꼬리. 데브넷은 `?cluster=devnet` 꼬리가 없으면
    /// 메인넷에서 찾아 «없는 거래» 로 뜬다.
    pub explorer_tx_prefix: &'static str,
    pub explorer_tx_suffix: &'static str,
}

/// 솔라나 데브넷. 기본 클러스터 — 데이터 파일이 접미사 없이 저장되는 "원본".
pub const SOLANA_DEVNET: ChainConfig = ChainConfig {
    chain_id: policy::SOLANA_DEVNET_ID,
    default_rpc: "https://api.devnet.solana.com",
    usdc_mint: solana::USDC_MINT_DEVNET,
    usdc_decimals: 6,
    genesis_hash: solana::GENESIS_DEVNET,
    x402_network_caip2: "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
    explorer_tx_prefix: "https://explorer.solana.com/tx/",
    explorer_tx_suffix: "?cluster=devnet",
};

/// 솔라나 메인넷(mainnet-beta, 실제 자금).
pub const SOLANA_MAINNET: ChainConfig = ChainConfig {
    chain_id: policy::SOLANA_MAINNET_ID,
    default_rpc: "https://api.mainnet-beta.solana.com",
    usdc_mint: solana::USDC_MINT_MAINNET,
    usdc_decimals: 6,
    genesis_hash: solana::GENESIS_MAINNET,
    x402_network_caip2: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp",
    explorer_tx_prefix: "https://explorer.solana.com/tx/",
    explorer_tx_suffix: "",
};

/// 사용자가 선택한 클러스터 번호. 폴백은 전부 데브넷 — 판정은 policy::chain_id_for.
fn selected_chain_id() -> u64 {
    env_chain_id().unwrap_or_else(settings_chain_id)
}

/// `BUDU_CHAIN_ID` 로 강제된 체인 ID (없으면 None). 잘못된 값이면 즉시 종료한다.
fn env_chain_id() -> Option<u64> {
    // 환경변수 우선 — 테스트·스크립트가 사용자의 라이브 settings.json 에 의존하지 않게 체인을 고정한다.
    // GUI 와 다른 (정상) 체인을 가리키더라도, 결제 요청에 각인된 chain_id 를 GUI 가 승인 시 대조해
    // 거부하므로 잘못된 체인으로 송금되지 않는다. 단, **잘못/미지원 값은 조용히
    // settings 로 폴백하지 않는다** — 오타(예: "84532x")가 사용자 의도와 다른 체인(메인넷=실돈)으로
    // 조용히 동작하는 footgun 을 막으려고, 명시 override 가 유효하지 않으면 즉시 종료한다.
    if let Some(v) = std::env::var("BUDU_CHAIN_ID")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        let id = v.parse::<u64>().unwrap_or_else(|_| {
            eprintln!("BUDU_CHAIN_ID is not an integer: {v:?}");
            std::process::exit(1);
        });
        if !policy::SUPPORTED_CHAIN_IDS.contains(&id) {
            eprintln!(
                "BUDU_CHAIN_ID={id} is not a supported cluster (supported: {} devnet / {} mainnet).",
                SOLANA_DEVNET.chain_id,
                SOLANA_MAINNET.chain_id,
            );
            std::process::exit(1);
        }
        return Some(id);
    }
    None
}

/// settings.json 이 말하는 체인 ID (환경변수를 보지 않는다).
fn settings_chain_id() -> u64 {
    // 단위 테스트는 실제 ~/.budu/settings.json 을 읽지 않는다 — 사용자의 지갑 설정(메인넷 등)에
    // 테스트 결과가 좌우되지 않게 기본 클러스터(데브넷)로 고정해 항상 결정론적이게 한다.
    #[cfg(test)]
    {
        SOLANA_DEVNET.chain_id
    }
    #[cfg(not(test))]
    {
        // 판정은 GUI(src-tauri chain.rs selected_chain_id)와 **같은 함수** `policy::chain_id_for`
        // — 두 프로세스가 여기서 어긋나면 잔액·결제가 서로 다른 클러스터를 본다. 폴백 갈래의 이유는 그쪽 주석.
        let Ok(dir) = crate::wallet::budu_dir() else {
            return policy::FALLBACK_CHAIN_ID; // 홈을 못 정함 = 못 읽음(보수적)
        };
        let file = policy::SettingsFile::read(&dir.join("settings.json"));
        policy::chain_id_for(&file, || policy::wallet_exists_in(&dir))
    }
}

/// **환경변수가 settings 와 다른 체인을 강제하고 있는가**.
///
/// `BUDU_CHAIN_ID` 는 체인만 바꾸고 `settings.json` 의 `rpc_url` 은 그대로 뒀다 → 커스텀 RPC
/// (또는 프리셋 RPC)를 쓰는 사람이 이 환경변수로 체인을 넘기면 **딴 체인의 RPC 에 이 체인의
/// 컨트랙트를 묻는 꼴**이 되어 잔액 조회가 `returned no data ("0x")` 로 죽는다.
/// GUI 로 체인을 바꾸면 프리셋이 새 체인으로 재매핑되므로 정상 — 환경변수 경로만의 함정이다.
///
/// 두 체인이 **같으면 false** — 그때 rpc_url 은 이 체인의 것이 맞으니 그대로 존중한다.
pub fn env_forces_other_chain() -> bool {
    match env_chain_id() {
        Some(id) => id != settings_chain_id(),
        None => false,
    }
}

/// 현재 활성 클러스터 — settings.json 의 chain_id 로 런타임 선택(GUI 와 동일 파일 → 자동 일치). 모르는 번호는 데브넷.
pub fn active_chain() -> ChainConfig {
    match selected_chain_id() {
        id if id == SOLANA_MAINNET.chain_id => SOLANA_MAINNET,
        _ => SOLANA_DEVNET,
    }
}

/// 체인별로 분리되는 데이터 파일 이름(history/x402_settlements) — 규칙은 `policy::chain_file_name`
/// (src-tauri 의 chain_file 과 같은 함수). 두 프로세스가 같은 settings.json 을 같은 판정으로 읽으므로
/// GUI 와 MCP 가 항상 같은 파일을 가리킨다. active_chain() 으로 정규화(미지원 id 는 데브넷 폴백)한
/// id 를 넘겨 활성 체인과 파일이 어긋나지 않게 한다.
pub fn chain_file(stem: &str) -> String {
    policy::chain_file_name(active_chain().chain_id, stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 상수 회귀 가드 — GUI(src-tauri/src/chain.rs)와 같은 값이어야 한다(민트·genesis 는 shared/solana.rs 한 곳).
    #[test]
    fn cluster_constants_are_pinned() {
        assert_eq!(SOLANA_DEVNET.chain_id, 103);
        assert_eq!(SOLANA_MAINNET.chain_id, 101);
        assert_eq!(SOLANA_DEVNET.usdc_mint, "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU");
        assert_eq!(SOLANA_MAINNET.usdc_mint, "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
        assert_eq!(SOLANA_DEVNET.usdc_decimals, 6);
        assert_eq!(SOLANA_MAINNET.usdc_decimals, 6);
        // 🔴 CAIP-2 = "solana:" + genesis 앞 32자 — 손으로 적은 값이 genesis 와 어긋나지 않게.
        for c in [SOLANA_DEVNET, SOLANA_MAINNET] {
            assert_eq!(c.x402_network_caip2, format!("solana:{}", &c.genesis_hash[..32]));
        }
        assert_ne!(SOLANA_DEVNET.x402_network_caip2, SOLANA_MAINNET.x402_network_caip2);
        // 테스트 빌드는 settings.json 을 안 읽고 데브넷 고정.
        assert_eq!(active_chain().chain_id, SOLANA_DEVNET.chain_id);
    }

    // policy::SUPPORTED_CHAIN_IDS 와 이 크레이트의 클러스터 셋(active_chain 의 match·env_chain_id 의 검사)이
    // 같아야 한다 — 한쪽에만 추가하면 그 클러스터의 커스텀 RPC 가 조용히 버려지거나 살아남는다.
    #[test]
    fn supported_chain_ids_match_this_crate() {
        let ours = [SOLANA_DEVNET.chain_id, SOLANA_MAINNET.chain_id];
        for id in policy::SUPPORTED_CHAIN_IDS {
            assert!(ours.contains(&id), "policy 엔 있는데 여기 없다: {id}");
        }
        for id in ours {
            assert!(policy::SUPPORTED_CHAIN_IDS.contains(&id));
        }
    }
}
