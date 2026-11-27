// 잔액 조회 — 활성 클러스터의 SOL + USDC (솔라나 JSON-RPC).
//
// 조회 자체는 shared/solana.rs(`fetch_balances`, MCP 와 같은 소스)가 하고, 여기는 활성 클러스터 값을 채워
// 부르고 실패 갈래를 사람 말로 바꾼다.

use crate::i18n::{tf, ts};
use serde::Serialize;

use crate::chain::active_chain;
use crate::settings::{effective_rpc, redact_urls};
use crate::solana::{self, BalanceError, BalanceQuery};

/// 잔액 — 보기 좋게 다듬기 전의 십진수 문자열.
/// `sol` = 네이티브(수수료) 토큰. x402 결제엔 필요 없다 — 보여 주기만.
/// `usdc` = 결제에 쓰는 돈. 내 USDC ATA 가 아직 없으면 **0**.
#[derive(Serialize)]
pub(crate) struct Balances {
    sol: String,
    usdc: String,
}

/// 실패 갈래 → 사람 말 (순수 — 테스트가 직접 부른다). URL 은 가린다(지정 RPC 의 API 키).
pub(crate) fn describe(e: BalanceError, want_genesis: &str) -> String {
    match e {
        // 실제로 본 원인은 EVM 시절 파일의 `0x…` 주소 — 앱의 「옛 형식 지갑」 화면이 비번 한 번으로 고친다.
        BalanceError::BadAddress => ts!(
            "지갑 주소가 올바르지 않아요 — 옛 형식 지갑이면 부두 앱을 열어 비밀번호를 넣으면 고쳐져요",
            "The wallet address isn't valid — if it's an old-format wallet, open the Budu app and enter your password to fix it"
        )
        .to_string(),
        BalanceError::WrongCluster(got) => {
            let got_name = solana::cluster_name_of(&got).unwrap_or("?");
            let want_name = solana::cluster_name_of(want_genesis).unwrap_or("?");
            tf!(
                "지정한 RPC 가 다른 클러스터({got_name})에 연결되어 있어요 — 고른 네트워크({want_name})와 달라서 잔액을 보여주지 않아요. 설정에서 RPC 를 비우거나 맞는 주소로 바꾸세요.",
                "The custom RPC is on a different cluster ({got_name}) than the network you picked ({want_name}), so the balance isn't shown. Clear the RPC in Settings or point it at the right cluster."
            )
        }
        // getGenesisHash 를 모르는 서버 = 솔라나 RPC 가 아니다(옛 EVM RPC 등) — «연결 실패» 로 부르면 오해한다.
        BalanceError::Genesis(solana::RpcError::Rpc { code: -32601, .. }) => ts!(
            "지정한 RPC 가 솔라나 RPC 가 아니에요. 설정에서 RPC 를 비우거나 솔라나 주소로 바꾸세요.",
            "The custom RPC isn't a Solana RPC. Clear it in Settings or use a Solana RPC address."
        )
        .to_string(),
        BalanceError::Genesis(e) => tf!(
            "RPC 연결 실패: {}",
            "Couldn't reach the RPC server: {}",
            redact_urls(&e.to_string())
        ),
        BalanceError::Sol(e) => tf!(
            "SOL 잔액 조회 실패: {}",
            "Couldn't read your SOL balance: {}",
            redact_urls(&e.to_string())
        ),
        BalanceError::Usdc(e) => tf!(
            "USDC 잔액 조회 실패: {}",
            "Couldn't read your USDC balance: {}",
            redact_urls(&e.to_string())
        ),
    }
}

/// 지갑 주소의 SOL + USDC 잔액을 활성 클러스터에서 조회한다.
/// 인자 이름 `addr_hex` 는 프론트 호출(`invoke("get_balances", { addrHex })`)과의 계약이라 그대로 둔다 — 값은 base58.
#[tauri::command]
pub(crate) async fn get_balances(addr_hex: String) -> Result<Balances, String> {
    let chain = active_chain();
    let rpc = effective_rpc();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| tf!("HTTP 준비 실패: {e}", "Couldn't set up HTTP: {e}"))?;
    let q = BalanceQuery {
        rpc_url: &rpc,
        owner: addr_hex.trim(),
        usdc_mint: chain.usdc_mint,
        usdc_decimals: chain.usdc_decimals,
        genesis_hash: chain.genesis_hash,
    };
    let raw = solana::fetch_balances(&client, &q)
        .await
        .map_err(|e| describe(e, chain.genesis_hash))?;
    Ok(Balances {
        sol: solana::format_units(raw.lamports, 9),
        usdc: solana::format_units(raw.usdc_units, chain.usdc_decimals),
    })
}

/// 받는 주소 문자열을 확인한다 (x402 결제 받는 쪽·신뢰 주소 공용). 솔라나 주소 = base58 32바이트.
/// 돌려주는 값은 다듬은(trim) 원문 — base58 은 대소문자를 가리므로 바꾸지 않는다.
pub(crate) fn parse_to_addr(to: &str) -> Result<String, String> {
    let t = to.trim();
    if solana::parse_pubkey(t).is_none() {
        return Err(ts!(
            "받는 주소가 올바른 솔라나 주소가 아니에요",
            "That recipient isn't a valid Solana address"
        )
        .into());
    }
    Ok(t.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solana::RpcError;

    #[test]
    fn parse_to_addr_accepts_base58_only() {
        let a = "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk";
        assert_eq!(parse_to_addr(&format!(" {a} ")).unwrap(), a);
        assert!(parse_to_addr("0x036CbD53842c5426634e7929541eC2318f3dCF7e").is_err());
        assert!(parse_to_addr("").is_err());
    }

    #[test]
    fn describe_redacts_and_names_clusters() {
        let m = describe(
            BalanceError::WrongCluster(solana::GENESIS_MAINNET.into()),
            solana::GENESIS_DEVNET,
        );
        assert!(m.contains("mainnet-beta") && m.contains("devnet"), "{m}");
        let m = describe(
            BalanceError::Usdc(RpcError::Http(
                "error sending request for url (https://rpc.example/v2/SECRET)".into(),
            )),
            solana::GENESIS_DEVNET,
        );
        assert!(!m.contains("SECRET"), "{m}");
        let m = describe(
            BalanceError::Genesis(RpcError::Rpc { code: -32601, message: "Method not found".into() }),
            solana::GENESIS_DEVNET,
        );
        assert!(m.contains("솔라나 RPC 가 아니") || m.contains("isn't a Solana RPC"), "{m}");
    }

    // 라이브 — 데브넷 공개 RPC 에 실제로 묻는다. `cargo test -- --ignored live_` 로만.
    // 팬텀 표준 문구(abandon…about)의 첫 계정은 데브넷에 USDC ATA 가 없을 가능성이 높다 → 0 이어도 성공.
    #[tokio::test]
    #[ignore]
    async fn live_devnet_balance() {
        let client = reqwest::Client::new();
        let q = BalanceQuery {
            rpc_url: crate::chain::SOLANA_DEVNET.default_rpc,
            owner: "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk",
            usdc_mint: solana::USDC_MINT_DEVNET,
            usdc_decimals: 6,
            genesis_hash: solana::GENESIS_DEVNET,
        };
        let raw = solana::fetch_balances(&client, &q).await.unwrap();
        eprintln!("{raw:?}");
        // 같은 질문을 메인넷 genesis 로 하면 클러스터 불일치로 거절돼야 한다.
        let q2 = BalanceQuery {
            genesis_hash: solana::GENESIS_MAINNET,
            ..q
        };
        assert!(matches!(
            solana::fetch_balances(&client, &q2).await,
            Err(BalanceError::WrongCluster(_))
        ));
    }
}
