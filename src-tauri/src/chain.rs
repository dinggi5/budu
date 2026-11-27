// 클러스터 설정 (EVM 체인 → 솔라나 데브넷/메인넷) — RPC·USDC 민트·genesis hash 를 ChainConfig 한 묶음으로.
// 활성 클러스터는 settings.json 의 chain_id 로 **런타임 선택**한다(데브넷↔메인넷 토글). 그 선택 판정은
// shared/policy.rs(`policy::chain_id_for`)가 정본이다 — settings.rs 의 Settings 읽기와 MCP(budu-mcp/src/chain.rs)가
// 같은 함수를 쓴다(「같은 파일을 다르게 읽는」 뿌리 제거). 번호(101/103)의 출처는 policy.rs 주석.

use crate::policy::{self, SettingsFile};
use crate::solana;
use crate::store::budu_dir;

/// 한 클러스터의 백엔드 설정 묶음 — 잔액 조회· x402 조립에 쓰는 값만 담는다.
/// (표시 이름·익스플로러 URL 같은 "보여주기" 값은 프론트 src/lib/chain.ts 와 MCP budu-mcp/src/chain.rs 가 각자 보유.)
#[derive(Clone, Copy)]
pub(crate) struct ChainConfig {
    /// settings.json·데이터 파일 이름·IPC 의 클러스터 번호 (policy::SOLANA_*_ID).
    pub(crate) chain_id: u64,
    /// 설정(rpc_url)이 비어 있을 때의 폴백 공개 RPC.
    pub(crate) default_rpc: &'static str,
    /// Circle USDC 민트 (base58).
    pub(crate) usdc_mint: &'static str,
    /// USDC 소수 자릿수 (6). 한도·내역·잔액이 이 값으로 단위를 맞춘다.
    pub(crate) usdc_decimals: u8,
    /// `getGenesisHash` 전체 값 — 지정 RPC 가 이 클러스터인지 대조한다.
    pub(crate) genesis_hash: &'static str,
}

/// 솔라나 데브넷. 기본 클러스터 — 데이터 파일이 접미사 없이 저장되는 「원본」.
pub(crate) const SOLANA_DEVNET: ChainConfig = ChainConfig {
    chain_id: policy::SOLANA_DEVNET_ID,
    default_rpc: "https://api.devnet.solana.com",
    usdc_mint: solana::USDC_MINT_DEVNET,
    usdc_decimals: 6,
    genesis_hash: solana::GENESIS_DEVNET,
};

/// 솔라나 메인넷(mainnet-beta, 실제 자금). 공개 RPC 는 요청 제한이 빡빡하다 — 실사용은 설정에서 RPC 를 지정.
pub(crate) const SOLANA_MAINNET: ChainConfig = ChainConfig {
    chain_id: policy::SOLANA_MAINNET_ID,
    default_rpc: "https://api.mainnet-beta.solana.com",
    usdc_mint: solana::USDC_MINT_MAINNET,
    usdc_decimals: 6,
    genesis_hash: solana::GENESIS_MAINNET,
};

tokio::task_local! {
    /// 한 작업(송금/서명/승인) 동안 고정된 체인 ID. with_pinned_chain 으로 설정하면 그 작업의 모든
    /// active_chain()/chain_file()/effective_rpc 호출이 이 값을 본다 — 작업 도중 settings.json 이
    /// 바뀌어도 흔들리지 않게(한 결제 안에서 장부·전송·서명 체인이 갈라지는 것 차단).
    static PINNED_CHAIN: u64;
}

/// 작업 단위로 체인을 고정해 fut 를 실행한다 — 진입 시 한 번 정한 chain_id 로 전체 작업을 묶는다.
/// 같은 태스크 내 .await 를 넘어 유지되므로(중간에 spawn 안 함), 장부·RPC·서명·내역이 같은 체인을 본다.
///
/// 🔴 안쪽 future 는 **힙에 올린다**(실측 크래시). 감쌀 때마다 안쪽을 통째로 품으면 층마다 4배씩 불어
/// (서명 본체 3KB → 승인 215KB, 디버그) 승인 직후 tokio 작업 스레드 스택(2MB)을 넘쳐 앱이 죽었다.
/// `async fn` 이 아니라 그냥 `fn` 이어야 인자 `fut` 를 상태에 한 벌 더 품지 않는다.
pub(crate) fn with_pinned_chain<F: std::future::Future>(
    chain_id: u64,
    fut: F,
) -> impl std::future::Future<Output = F::Output> {
    PINNED_CHAIN.scope(chain_id, Box::pin(fut))
}

/// 사용자가 선택한 체인 ID. 작업이 체인을 고정했으면 그 값, 아니면 settings.json 을
/// `policy::chain_id_for` 로 읽는다(폴백 네 갈래의 이유는 그쪽 주석). settings::read_settings 가
/// 같은 함수로 같은 파일을 읽으므로 화면이 그리는 체인과 돈이 나가는 체인은 갈릴 수 없다.
fn selected_chain_id() -> u64 {
    if let Ok(id) = PINNED_CHAIN.try_with(|id| *id) {
        return id; // 작업이 고정한 체인 — 도중에 settings 가 바뀌어도 불변
    }
    let Ok(dir) = budu_dir() else {
        return policy::FALLBACK_CHAIN_ID; // 홈을 못 정함 = 못 읽음(보수적)
    };
    policy::chain_id_for(&SettingsFile::read(&dir.join("settings.json")), || {
        policy::wallet_exists_in(&dir)
    })
}

/// 체인 ID로 ChainConfig 를 찾는다. 미지원이면 None. (set_settings 가 들어온 설정의 chain_id 를
/// 그 체인 기준으로 검증할 때 쓴다 — 저장 전 활성 체인이 아니라.)
pub(crate) fn chain_by_id(id: u64) -> Option<ChainConfig> {
    match id {
        id if id == SOLANA_DEVNET.chain_id => Some(SOLANA_DEVNET),
        id if id == SOLANA_MAINNET.chain_id => Some(SOLANA_MAINNET),
        _ => None,
    }
}

/// 현재 활성 클러스터 — settings.json 의 chain_id 로 런타임 선택. 알 수 없는 ID는 데브넷으로 폴백.
pub(crate) fn active_chain() -> ChainConfig {
    chain_by_id(selected_chain_id()).unwrap_or(SOLANA_DEVNET)
}

/// 체인별로 분리해야 하는 데이터 파일 이름(spend/history/x402_settlements/trusted) — 규칙은
/// `policy::chain_file_name`(MCP 와 같은 함수). **active_chain() 으로 정규화한 id** 를 넘긴다(미지원/손상
/// chain_id 는 데브넷으로) — 그래야 알 수 없는 id 가 들어와도 활성 클러스터(데브넷 폴백)과 데이터
/// 파일이 어긋나 한도·신뢰목록을 조용히 리셋하지 않는다.
pub(crate) fn chain_file(stem: &str) -> String {
    policy::chain_file_name(active_chain().chain_id, stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 상수 회귀 가드 — 민트·genesis 오타는 「잔액이 늘 0」이나 「딴 클러스터에 결제」로 조용히 나간다.
    // 값의 출처: 민트 = Circle 문서, genesis = 공개 RPC `getGenesisHash` 실응답.
    #[test]
    fn cluster_constants_are_pinned() {
        assert_eq!(SOLANA_DEVNET.chain_id, 103);
        assert_eq!(SOLANA_MAINNET.chain_id, 101);
        assert_eq!(SOLANA_DEVNET.usdc_mint, "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU");
        assert_eq!(SOLANA_MAINNET.usdc_mint, "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
        assert_eq!(SOLANA_DEVNET.genesis_hash, "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG");
        assert_eq!(SOLANA_MAINNET.genesis_hash, "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d");
        assert_eq!(SOLANA_DEVNET.usdc_decimals, 6);
        assert_eq!(SOLANA_MAINNET.usdc_decimals, 6);
        // 미지원(옛 EVM 번호 포함) → None → active_chain/chain_file 이 데브넷으로 폴백.
        assert!(chain_by_id(8453).is_none());
        assert!(chain_by_id(84_532).is_none());
        assert_eq!(chain_by_id(101).unwrap().chain_id, 101);
        assert_eq!(chain_by_id(103).unwrap().chain_id, 103);
    }

    // policy::SUPPORTED_CHAIN_IDS 와 chain_by_id 가 같은 집합이어야 한다 — policy 는 이 목록으로
    // 「chain_id 를 못 알아보는 파일의 지정 RPC 를 버릴지」 정한다.
    #[test]
    fn supported_chain_ids_match_chain_by_id() {
        for id in policy::SUPPORTED_CHAIN_IDS {
            assert!(
                chain_by_id(id).is_some(),
                "policy 엔 있는데 chain_by_id 엔 없다: {id}"
            );
        }
        for c in [SOLANA_DEVNET, SOLANA_MAINNET] {
            assert!(policy::SUPPORTED_CHAIN_IDS.contains(&c.chain_id));
        }
        assert_eq!(policy::FALLBACK_CHAIN_ID, SOLANA_DEVNET.chain_id);
    }
}
