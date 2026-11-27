// x402 HTTP 결제 루프 — V2 · 솔라나 `exact`.
//
// 흐름:
//   1. 리소스 요청 → 서버가 402 + `PAYMENT-REQUIRED` 헤더(base64 JSON: accepts[] — 얼마를 누구에게 어느 토큰으로)
//   2. 우리가 처리할 수 있는 요구를 **닫힌 쪽으로** 고른다
//   3. GUI 에 결제 조건(`policy::SvmTerms`)을 넘겨 사람 승인 → GUI 가 트랜잭션을 조립·자체 검증·부분 서명해 돌려준다
//   4. `PAYMENT-SIGNATURE` = base64({x402Version:2, resource, accepted(고른 요구 원본), payload:{transaction}}) 로 같은 URL 재요청
//   5. 판매자(+페이실리테이터)가 검증 → 수수료 서명을 더해 제출 → 200 + `PAYMENT-RESPONSE`
//
// 비밀은 여기 없다. 서명은 GUI 프로세스만(payment IPC), MCP 는 HTTP·헤더 조립만 한다.
// V1(본문 요구·`X-PAYMENT`)과 EVM 요구는 버렸다 — 솔라나 exact 는 V2 로만 정의돼 있다.

use crate::tf;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde_json::Value;

use crate::chain::active_chain;
use crate::policy::{is_hidden_char, SvmSigned, SvmTerms};
use crate::solana;

/// 우리가 지원하는 결제 스킴.
pub const SCHEME: &str = "exact";

/// `extra` 에서 우리가 아는 칸. 이 밖의 칸이 있으면 **결제 조건일 수 있으니** 거절한다.
const KNOWN_EXTRA: &[&str] = &[
    "feePayer",
    "memo",
    "recentBlockhash",
    "lastValidBlockHeight",
    "transactionVersions",
    "splits",
    "ataCreationRequired",
];

/// 요구 최상위에서 우리가 아는 칸. V2 `PaymentRequirements` 의 일곱 칸 + 결제와 상관없는 설명 칸(V1 시절 이름).
/// 공식 미들웨어(@x402/express 2.30.0)가 실제로 내는 칸은 앞의 일곱 개다(budu-port 실측). 이 밖의 칸은 **결제 조건일 수 있으니** 거절한다.
const KNOWN_TOP: &[&str] = &[
    "scheme",
    "network",
    "amount",
    "asset",
    "payTo",
    "maxTimeoutSeconds",
    "extra",
    "description",
    "mimeType",
    "resource",
    "outputSchema",
];

/// 시간이 지나도 바뀔 수 있는 `extra` 힌트 — 다시 받은 402 와 대조할 때 뺀다(우리 tx 는 이 힌트를 쓰지 않는다).
const VOLATILE_EXTRA: &[&str] = &["recentBlockhash", "lastValidBlockHeight"];

/// 402 결제 요구 전체(원본 JSON 보존) — 제출 때 `resource` 를 그대로 에코한다.
pub struct PaymentRequired {
    raw: Value,
    pub version: u8,
}

/// accepts[] 중 우리가 고른 결제 요구 1건. raw = 원본 항목(제출 때 `accepted` 로 통째로 에코).
pub struct Requirement {
    pub raw: Value,
    /// base unit 금액("10000").
    pub amount: String,
    pub pay_to: String,
    /// 판매자가 이 요구를 유효하다고 보는 시간(초). 승인이 이보다 오래 걸리면 제출 전에 402 를 다시 받아 같은 조건인지 본다.
    pub max_timeout_secs: u64,
    /// GUI 에 넘길 결제 조건.
    pub terms: SvmTerms,
}

/// 조립된 결제 제출(헤더 이름 + base64 값 + 정산 응답을 읽을 헤더 이름).
pub struct Submission {
    pub header_name: &'static str,
    pub value: String,
    pub response_header: &'static str,
    /// 같은 값을 함께 실어 줄 두 번째 헤더 — 솔라나 V2 에선 쓰지 않는다(None).
    pub alt_header: Option<&'static str>,
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// 402 응답에서 결제 요구를 추출한다. V2 는 `payment-required` 헤더(base64 JSON). 헤더가 없으면 본문(JSON)을 본다 —
/// 본문에 V2 모양을 싣는 서버도 있다. 버전은 고를 때 2 인지 본다.
pub fn parse_required(header: Option<&str>, body: &str) -> Result<PaymentRequired, String> {
    let raw: Value = match header.map(str::trim).filter(|s| !s.is_empty()) {
        Some(h) => {
            let bytes = B64.decode(h).map_err(|e| {
                tf!(
                    "payment-required 헤더 base64 디코드 실패: {e}",
                    "Couldn't base64-decode the payment-required header: {e}"
                )
            })?;
            serde_json::from_slice(&bytes).map_err(|e| {
                tf!(
                    "payment-required 헤더 JSON 파싱 실패: {e}",
                    "Couldn't parse the payment-required header as JSON: {e}"
                )
            })?
        }
        None => serde_json::from_str(body).map_err(|e| {
            tf!(
                "402 본문 파싱 실패: {e}",
                "Couldn't parse the 402 body: {e}"
            )
        })?,
    };
    let version = raw.get("x402Version").and_then(Value::as_u64).unwrap_or(1).min(255) as u8;
    Ok(PaymentRequired { raw, version })
}

/// accepts[] 중 우리가 처리할 수 있는 요구를 고른다(활성 클러스터 기준).
pub fn pick_requirement(pr: &PaymentRequired) -> Result<Requirement, String> {
    pick_requirement_for(pr, &active_chain())
}


/// 서버가 준 짧은 칸을 사람·AI 에게 보일 만큼만 — 제어문자·방향/서식 문자를 빼고 64자에서 자른다.
fn clip(s: &str) -> String {
    let kept: Vec<char> = s.chars().filter(|&c| !is_hidden_char(c)).collect();
    let t: String = kept.iter().take(64).collect();
    if kept.len() > 64 { format!("{t}…") } else { t }
}

/// 요구 하나를 솔라나 exact 조건으로 검사한다 — 통과하면 결제 조건, 아니면 거절 이유(짧은 영어 코드, AI·로그용).
///
/// 🔴 **닫힌 쪽**: 우리가 아는 모양만 받는다. 규칙:
/// scheme `exact` · network = 활성 클러스터 CAIP-2 · asset = 그 클러스터 USDC 민트(정확 일치) · amount = 0 보다 큰 u64 정수 ·
/// payTo·feePayer = base58 32바이트 · feePayer 필수 · memo ≤ 256바이트 · transactionVersions 에 0 · splits 없음 ·
/// ataCreationRequired 아님 · `extra` 에 모르는 칸 없음 · 최상위에 모르는 칸 없음 · maxTimeoutSeconds = 0 보다 큰 정수.
fn check_requirement(e: &Value, chain: &crate::chain::ChainConfig) -> Result<Requirement, &'static str> {
    let top = e.as_object().ok_or("shape")?;
    if top.keys().any(|k| !KNOWN_TOP.contains(&k.as_str())) {
        return Err("unknown");
    }
    if str_field(e, "scheme") != Some(SCHEME) {
        return Err("scheme");
    }
    if str_field(e, "network") != Some(chain.x402_network_caip2) {
        return Err("network");
    }
    if str_field(e, "asset") != Some(chain.usdc_mint) {
        return Err("asset");
    }
    let amount = str_field(e, "amount").ok_or("amount")?;
    let ok_amount = !amount.is_empty()
        && amount.bytes().all(|b| b.is_ascii_digit())
        && !(amount.len() > 1 && amount.starts_with('0'))
        && amount.parse::<u64>().is_ok_and(|a| a > 0);
    if !ok_amount {
        return Err("amount");
    }
    // 스펙(V2)상 필수 칸이다. 없거나 0·소수·음수면 거절 — 「얼마나 기다려 주나」 를 모르면 늦은 제출을 판단할 수 없다.
    let max_timeout_secs = e
        .get("maxTimeoutSeconds")
        .and_then(Value::as_u64)
        .filter(|&t| t > 0)
        .ok_or("maxTimeoutSeconds")?;
    let pay_to = str_field(e, "payTo").filter(|p| solana::parse_pubkey(p).is_some() && !p.contains(char::is_whitespace)).ok_or("payTo")?;
    let extra = e.get("extra").and_then(Value::as_object).ok_or("extra.feePayer")?;
    if extra.keys().any(|k| !KNOWN_EXTRA.contains(&k.as_str())) {
        return Err("extra.unknown");
    }
    let fee_payer = extra
        .get("feePayer")
        .and_then(Value::as_str)
        .filter(|p| solana::parse_pubkey(p).is_some() && !p.contains(char::is_whitespace))
        .ok_or("extra.feePayer")?;
    let memo = match extra.get("memo") {
        None | Some(Value::Null) => None,
        // 제어·방향 문자가 든 메모는 거절한다 — tx 에 그대로 새겨지고 승인 창에 그려지는 값이라 고쳐 쓰지 않는다.
        Some(Value::String(m))
            if !m.is_empty() && m.len() <= 256 && !m.chars().any(is_hidden_char) =>
        {
            Some(m.clone())
        }
        Some(_) => return Err("extra.memo"),
    };
    match extra.get("transactionVersions") {
        None => {}
        Some(Value::Array(vs)) if vs.iter().any(|v| v.as_u64() == Some(0) && v.is_u64()) => {}
        Some(_) => return Err("extra.transactionVersions"),
    }
    match extra.get("splits") {
        None | Some(Value::Null) => {}
        Some(Value::Array(a)) if a.is_empty() => {}
        Some(_) => return Err("extra.splits"),
    }
    match extra.get("ataCreationRequired") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => {}
        Some(_) => return Err("extra.ataCreationRequired"),
    }
    Ok(Requirement {
        raw: e.clone(),
        amount: amount.to_string(),
        pay_to: pay_to.to_string(),
        max_timeout_secs,
        terms: SvmTerms {
            network: chain.x402_network_caip2.to_string(),
            asset: chain.usdc_mint.to_string(),
            pay_to: pay_to.to_string(),
            amount: amount.to_string(),
            fee_payer: fee_payer.to_string(),
            memo,
        },
    })
}

/// `pick_requirement` 를 **정한 클러스터로** — 결제 흐름은 클러스터를 한 번 읽어 고르기부터 요청 각인까지 그 값 하나로 간다.
/// 통과하는 첫 요구를 고른다. 하나도 없으면 서버가 제시한 것과 각각의 거절 이유를 (잘라서) AI 에게 돌려준다.
pub fn pick_requirement_for(pr: &PaymentRequired, chain: &crate::chain::ChainConfig) -> Result<Requirement, String> {
    if pr.version != 2 {
        return Err(tf!(
            "x402 V{} 요구예요 — 부두는 솔라나 x402 V2 만 결제해요. 아무것도 결제하지 않았습니다.",
            "This is an x402 V{} challenge — Budu only pays Solana x402 V2. Nothing was paid.",
            pr.version
        ));
    }
    let list = pr.raw.get("accepts").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut offered = Vec::new();
    for e in list.iter().take(16) {
        match check_requirement(e, chain) {
            Ok(r) => return Ok(r),
            Err(why) => {
                // 서버가 보낸 문자열 — 칸마다 길이를 자르고 제어문자를 빼서 AI 에게 넘긴다.
                let f = |k: &str| clip(str_field(e, k).unwrap_or("?"));
                if offered.len() < 8 {
                    offered.push(format!("{}/{}/{} ✗{why}", f("scheme"), f("network"), f("asset")));
                }
            }
        }
    }
    Err(tf!(
        "이 서버의 결제 조건 중 부두가 처리할 수 있는 게 없어요(필요: exact · {} · USDC). 아무것도 결제하지 않았습니다. 서버 제시: [{}]",
        "None of this server's payment options can be handled by Budu (needs exact · {} · USDC). Nothing was paid. Server offered: [{}]",
        chain.x402_network_caip2,
        offered.join(", ")
    ))
}

/// 다시 받은 402 에서 **우리가 서명한 것과 같은 요구**를 찾는다.
/// 같다 = 힌트(`VOLATILE_EXTRA`)를 뺀 모든 칸이 JSON 으로 같다. 찾으면 그 **새** 원본을 돌려준다 — 판매자는 제출의 `accepted` 를
/// 자기 현재 요구와 통째로 대조하므로 힌트가 바뀌었으면 새 값을 에코해야 한다(우리 tx 는 힌트를 안 쓰니 서명은 그대로 유효).
pub fn find_same_offer(pr: &PaymentRequired, picked: &Value) -> Option<Value> {
    fn strip(v: &Value) -> Value {
        let mut v = v.clone();
        if let Some(extra) = v.get_mut("extra").and_then(Value::as_object_mut) {
            for k in VOLATILE_EXTRA {
                extra.remove(*k);
            }
        }
        v
    }
    if pr.version != 2 {
        return None;
    }
    let want = strip(picked);
    pr.raw
        .get("accepts")
        .and_then(Value::as_array)?
        .iter()
        .take(16)
        .find(|e| strip(e) == want)
        .cloned()
}

impl PaymentRequired {
    /// 결제 사유 후보(없으면 빈 문자열): 요구별 설명(V1) > 최상위 설명(V2).
    pub fn description(&self, req: &Requirement) -> String {
        if let Some(s) = str_field(&req.raw, "description") {
            if !s.trim().is_empty() {
                return s.trim().to_string();
            }
        }
        self.raw
            .get("resource")
            .and_then(|r| r.get("description"))
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    }

    /// 부분 서명 트랜잭션을 제출 헤더로 조립한다:
    ///   PAYMENT-SIGNATURE = base64({x402Version:2, resource, accepted=고른 요구 원본, payload:{transaction}})
    pub fn build_submission(&self, req: &Requirement, signed: &SvmSigned) -> Result<Submission, String> {
        let body = serde_json::json!({
            "x402Version": 2,
            "resource": self.raw.get("resource").cloned().unwrap_or(Value::Null),
            "accepted": req.raw,
            "payload": { "transaction": signed.transaction },
        });
        let bytes = serde_json::to_vec(&body).map_err(|e| {
            tf!(
                "payload 직렬화 실패: {e}",
                "Couldn't serialize the payload: {e}"
            )
        })?;
        Ok(Submission {
            header_name: "PAYMENT-SIGNATURE",
            value: B64.encode(bytes),
            response_header: "PAYMENT-RESPONSE",
            alt_header: None,
        })
    }
}

/// base unit 정수 문자열("10000")을 USDC 십진 문자열("0.01")로 — 사람 표시·송금 한도 검사용.
pub fn base_units_to_usdc(base: &str) -> Result<String, String> {
    let dec = active_chain().usdc_decimals as usize;
    let scale = 10u128.pow(dec as u32);
    let n: u128 = base.trim().parse().map_err(|_| {
        tf!(
            "금액 형식 오류: {base}",
            "That amount isn't a valid number: {base}"
        )
    })?;
    let whole = n / scale;
    let frac = n % scale;
    if frac == 0 {
        Ok(whole.to_string())
    } else {
        let s = format!("{whole}.{frac:0width$}", width = dec);
        Ok(s.trim_end_matches('0').trim_end_matches('.').to_string())
    }
}

/// 정산 응답(PAYMENT-RESPONSE, base64 JSON)에서 정산 tx id 와 성공여부를 뽑는다.
/// V2 표준은 "transaction"(솔라나 = feePayer 서명 base58). 옛 구현의 "txHash" 도 본다.
/// tx 해시를 못 찾으면 None(표시할 게 없음).
pub fn parse_settlement(b64: &str) -> Option<(String, bool)> {
    let bytes = B64.decode(b64.trim()).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    let tx = v
        .get("transaction")
        .or_else(|| v.get("txHash"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let success = v.get("success").and_then(Value::as_bool).unwrap_or(true);
    Some((tx, success))
}

/// 솔라나 tx id(= 64바이트 서명의 base58) 꼴인가 — 판매자가 준 정산 tx 를 탐색기 링크로 만들어도 되는 꼴인지.
pub fn is_tx_hash(s: &str) -> bool {
    solana::is_signature(s)
}

/// 정산 증빙 헤더가 **명시적으로 실패**(`success:false`)를 말하나 — tx 칸이 비어 있어도 본다.
/// `parse_settlement` 은 tx 가 없으면 None 이라 이 말을 놓친다. 못 읽는 헤더는 false(말한 게 없다).
pub fn settlement_says_failed(b64: &str) -> bool {
    B64.decode(b64.trim())
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.get("success").and_then(Value::as_bool))
        == Some(false)
}

/// 서명한 트랜잭션을 실은 재요청이 **응답 없이** 끝났을 때의 안내. 서버에 닿았는지 모른다 —
/// 닿았다면 페이실리테이터가 정산했을 수 있다. 「다시 부르면 새 트랜잭션이 하나 더 나간다」가 핵심이다.
pub fn signed_unknown_notice(err: &str) -> String {
    tf!(
        "서명한 결제 트랜잭션을 보냈는데 응답을 받지 못했어요({err}). 서버가 이미 정산했을 수 있습니다 — \
         **다시 요청하면 두 번 결제될 수 있어요.** 다시 시도하기 전에 사용자에게 알리고 지갑 내역(정산 여부)을 확인하세요.",
        "The signed payment transaction was sent but no reply came back ({err}). The server may already have \
         settled it — **asking again may pay twice.** Tell the user and check the wallet history (settlement) before retrying."
    )
}

#[cfg(test)]
mod tests;
