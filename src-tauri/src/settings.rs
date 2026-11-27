// 사용자 설정 (Session 7~) — 한도·자율 결제·RPC·잠금 동작. ~/.budu/settings.json 영속화.

use crate::i18n::ts;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::chain::{active_chain, chain_by_id};
use crate::limits::parse_usdc_nonneg;
use crate::policy::{self, SettingsFile};
use crate::store::{budu_dir, write_json};

/// 한도 기본값 (사용자가 설정 화면에서 조정 가능). 권한 모델의 가드레일.
/// 신규 기본 체인은 메인넷이라 이 값이 곧 **첫 사용자의 실돈 한도**다 — USDC 5/20 은 «AI 소액 결제 가드레일» 로 적정하다. (원본 지갑의 ETH 한도 칸은 읽을 때 무시되고 다음 저장 때 빠진다.)
const DEFAULT_SINGLE_USDC: &str = "5";
const DEFAULT_DAILY_USDC: &str = "20";

/// 자율 결제 기본값. 자율 한도 0 = 자율 결제 꺼짐 = 항상 사람 비번 승인(=기존 동작).
/// 보호자가 설정에서 켜야만 작동한다 → 디폴트는 보안 우선.
const DEFAULT_AUTO_APPROVE_USDC: &str = "0";
/// 세션 자동 잠금까지 유휴 시간(분). 0 = 유휴 잠금 안 함(앱 종료·긴급 잠금 시엔 항상 잠김).
const DEFAULT_AUTO_LOCK_MINS: &str = "30";
/// 설정 화면 자동 잠금 목록(분). 0 = 잠그지 않음. 목록 밖의 옛 값은 읽을 때 `snap_lock_mins` 가 짧은 쪽으로 접는다.
pub(crate) const LOCK_CHOICES: [u64; 7] = [10, 30, 60, 240, 1440, 10080, 0];

/// 목록 밖의 자동 잠금 값을 **가장 가까운 짧은 쪽**으로(잠금이 늦어지지 않게). 0·목록 값은 그대로.
/// 10분보다 짧은 값(1~9)은 더 짧은 목록 값이 없어 그대로 둔다 — 화면이 그 값을 한 칸 더 보여 준다.
/// 숫자가 아니면 손대지 않는다(저장 검증이 따로 막는다).
fn snap_lock_mins(v: &str) -> String {
    let Ok(n) = v.trim().parse::<u64>() else {
        return v.to_string();
    };
    if n == 0 || n < 10 || LOCK_CHOICES.contains(&n) {
        return n.to_string();
    }
    let down = LOCK_CHOICES.iter().copied().filter(|&c| c != 0 && c <= n).max().unwrap_or(10);
    down.to_string()
}

/// 네트워크 하나의 한도 묶음 — 결제당·하루·자율 결제 금액. 체인마다 따로 둔다
/// (사용액·내역·화이트리스트가 이미 체인별이라, 한도만 공통이면 데브넷 연습 값이 메인넷 실돈에 그대로 걸린다).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub(crate) struct NetLimits {
    pub(crate) single_usdc: String,
    pub(crate) daily_usdc: String,
    pub(crate) auto_approve_usdc: String,
}

/// **chain_id 필드가 없는 설정 파일**의 serde 기본값 — `policy::FALLBACK_CHAIN_ID`(데브넷).
/// 신규 기본(`Settings::default()`, 파일 자체가 없음)과는 다른 질문이다 — «필드가 없는 옛 파일» 의 해석은
/// 언제나 데브넷이어야 한다. 언젠가 신규 기본을 메인넷으로 바꾸더라도 여기를 따라 바꾸면 앱 업데이트 한 번에
/// 옛 사용자가 조용히 실돈 체인으로 옮겨진다.
///
/// 읽기 경로(`read_settings`)는 이 값을 쓰지 않는다 — 체인은 늘 `policy::chain_id_for` 가 정한다
/// (같은 답: 필드 없음 → 테스트넷). 이 기본값이 실제로 쓰이는 곳은 **다시 저장할 목적**으로 파일을
/// 그대로 읽는 `read_settings_for_update` 다(옛 파일을 저장해도 테스트넷이 명시적으로 박히게).
fn default_chain_id() -> u64 {
    policy::FALLBACK_CHAIN_ID
}

/// 사용자 조정 가능한 한도 설정 (십진수 문자열). 권한 모델의 가드레일.
///
/// 🔴 한도 셋(single·daily·auto_approve)의 정본은 **`limits`(체인별)** 다. 평평한 세 칸은 읽을 때
/// `normalize` 가 활성 체인 값으로 채우는 **보기 칸**이다 — 결제·세션·홈은 지금처럼 평평한 칸을 읽으면 된다.
/// 평평한 칸은 파일에도 함께 쓴다(옛 빌드로 되돌려도 활성 체인 한도가 그대로 걸리게).
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct Settings {
    pub(crate) single_usdc: String,
    pub(crate) daily_usdc: String,
    /// 자율 결제 한도 (USDC, 십진수). 이 금액 이하의 AI 결제 요청은 세션이 잠금 해제돼 있으면
    /// 비번 없이 자동 승인된다. "0"이면 자율 결제 꺼짐(항상 사람 비번). 옛 설정 파일엔 없어서 기본 "0".
    #[serde(default = "default_auto_approve_usdc")]
    pub(crate) auto_approve_usdc: String,
    /// 세션 자동 잠금 유휴 시간(분). "0"이면 유휴 잠금 안 함. 옛 파일 호환 위해 기본값 지정.
    #[serde(default = "default_auto_lock_mins")]
    pub(crate) auto_lock_mins: String,
    /// 잔액 조회·송금에 쓸 RPC 엔드포인트. 사용자가 프라이버시·속도 이유로 바꿀 수 있다.
    /// **빈 값 = "활성 체인의 공식 RPC를 따라간다"** (effective_rpc 가 active_chain().default_rpc 로 해석).
    /// 읽기는 chain_id 처럼 `policy::rpc_url_for` 가 정본이다 — read_settings 가 어느 갈래든
    /// 이 필드를 그 답으로 덮는다(MCP 와 같은 함수).
    /// 기본을 구체 URL 로 박아 저장하면 active_chain() 을 메인넷으로 바꿔도 옛 RPC 에 고정되는
    /// 함정이 생긴다 → 기본은 빈 값으로 둬서 체인 전환을 자동으로 따라가게 한다.
    #[serde(default)]
    pub(crate) rpc_url: String,
    /// 자리비움 자동 잠금: 창이 포커스를 잃으면(다른 앱 전환·화면 잠금) 세션을 즉시 잠근다. 기본 꺼짐.
    #[serde(default)]
    pub(crate) lock_on_blur: bool,
    /// 자율 결제 알림: 비번 없이 자동 승인된 결제를 OS 알림으로 사후 통지. 기본 켜짐
    /// (자율 = 보호자가 모르는 새 돈이 나가는 유일한 경로라, 끄는 쪽이 명시적 선택이어야 함).
    #[serde(default = "default_true")]
    pub(crate) notify_auto: bool,
    /// 자율 결제 알림에서 금액 숨기기 (프라이버시). macOS 알림은 잠금 화면에도
    /// 뜨고 화면 공유·녹화에도 잡힌다 — 켜면 제목이 금액 없이 "자율 결제"로만 나간다.
    /// 기본 꺼짐(기존 동작 보존 — 금액이 한눈에 보이는 쪽이 사후 인지엔 더 낫다).
    #[serde(default)]
    pub(crate) notify_hide_amount: bool,
    /// 자율 결제는 신뢰 주소(비번으로 승인한 적 있는 받는 주소)만. 기본 켜짐 —
    /// 끄면 한도 이하 금액이면 처음 보는 주소에도 비번 없이 나간다.
    #[serde(default = "default_true")]
    pub(crate) auto_trusted_only: bool,
    /// 활성 클러스터 번호 — 데브넷(103) ↔ 메인넷(101) 런타임 전환. chain::active_chain() 이 이 값을 읽는다.
    /// 옛 설정엔 없어서 기본 = 테스트넷(실돈 안전). 체인별로 사용액·내역·신뢰목록 파일이 분리된다.
    #[serde(default = "default_chain_id")]
    pub(crate) chain_id: u64,
    // ── 아래 둘은 **앱이 관리하는 필드**다. 설정 화면 폼에서 오는 값이 아니라서
    //    set_settings 가 클라이언트가 보낸 값을 무시하고 디스크 값을 지킨다(preserve_managed).
    //    프론트 Settings 타입엔 이 둘이 없으므로, 안 지키면 "저장하고 닫기" 한 번에 기본값으로 덮인다.
    /// 로그인 시 자동 시작 **희망값**.
    ///
    /// OS 로그인 아이템(LaunchAgent)만 보면 "사용자가 껐다"와 "패키지 관리자가 지웠다"를 구별할
    /// 수 없다. 캐스크의 `uninstall launchctl:` 은 `brew upgrade`·`brew reinstall` 때도 돌아서
    /// plist 를 내린다 → 업데이트 한 번에 자동 시작이 조용히 꺼진다.
    /// 그래서 희망값을 여기 따로 적어 두고, 시작할 때 대조한다(autostart::reconcile).
    ///
    /// None = 아직 모름(이 필드가 없던 시절의 설정 파일) → 첫 실행에서 OS 상태를 그대로 채택한다.
    /// **기본을 false 로 두면 안 된다** — 자동 시작을 켜 둔 기존 사용자가 "꺼짐을 원했다"로
    /// 기록되고, 그 뒤 진짜로 꺼졌을 때 복구 대상에서 빠진다.
    #[serde(default)]
    pub(crate) autostart: Option<bool>,

    /// 화면 언어. `None` = 아직 안 고름 → 시스템 언어를 따라간다(i18n::init).
    ///
    /// **기본을 "ko" 로 박지 않는다** — 그러면 영어 맥에서 첫 실행이 한국어로 열리고,
    /// 사용자가 언어를 고르지도 않았는데 "골랐다"로 기록돼 시스템 언어를 영영 안 따라간다.
    /// 폼에서 오는 값이 아니라 앱 관리 필드다(set_lang 이 따로 저장 — preserve_managed).
    #[serde(default)]
    pub(crate) lang: Option<String>,

    /// 메뉴바에 아이콘을 둘까. 기본 켬 — 옛 파일(필드 없음)의 주인은 메뉴바로 써 온 사람이라
    /// 그대로 두고, 신규도 켬(잔액·작업 예산을 메뉴바에서 한눈에). 끄면 앱은 도크와 큰 창으로만 쓴다.
    /// 폼에서 오는 값이 아니라 앱 관리 필드다(set_menubar 가 따로 저장하고 그 자리에서 아이콘을 켜고 끈다).
    #[serde(default = "default_true")]
    pub(crate) menubar: bool,

    /// 체인별 한도(키 = chain_id 문자열, 예 "101"). 옛 파일엔 없다 → 읽을 때 평평한 칸의 값을
    /// **모든 체인에 복사**한다(누구의 한도도 몰래 풀리지 않게). 폼에서 오는 값이다(`limits.<id>.<칸>` 으로 합침).
    #[serde(default)]
    pub(crate) limits: BTreeMap<String, NetLimits>,

    /// 사람이 비번으로 승인한 x402 받는 주소를 화이트리스트에 저절로 넣을까. 기본 꺼짐 —
    /// 화이트리스트는 사용자가 직접 넣은 것만. 이미 학습된 주소는 그대로 남는다.
    #[serde(default)]
    pub(crate) auto_learn_trusted: bool,
}

fn default_true() -> bool {
    true
}

fn default_auto_approve_usdc() -> String {
    DEFAULT_AUTO_APPROVE_USDC.to_string()
}

fn default_auto_lock_mins() -> String {
    DEFAULT_AUTO_LOCK_MINS.to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            single_usdc: DEFAULT_SINGLE_USDC.into(),
            daily_usdc: DEFAULT_DAILY_USDC.into(),
            auto_approve_usdc: DEFAULT_AUTO_APPROVE_USDC.into(),
            auto_lock_mins: DEFAULT_AUTO_LOCK_MINS.into(),
            rpc_url: String::new(), // 빈 값 = 활성 체인 공식 RPC 따라감 (effective_rpc 가 해석)
            lock_on_blur: false,
            notify_auto: true,
            notify_hide_amount: false,
            auto_trusted_only: true,
            // 신규 기본 = 메인넷(`policy::NEW_INSTALL_CHAIN_ID`, 개발 14). 내려받는 사람은 쓰려고 받는다.
            // 옛 파일(필드 없음)은 default_chain_id 가, 깨진 파일과 «설정 파일만 없는 기존 지갑» 은
            // conservative() 가 따로 맡는다 — 여기는 **진짜 신규**(지갑도 설정도 없는 첫 실행)만.
            chain_id: policy::NEW_INSTALL_CHAIN_ID, // 메인넷 (policy.rs 주석)
            autostart: None,    // 아직 모름 → 첫 실행에서 OS 상태를 채택
            lang: None,         // 아직 안 고름 → 시스템 언어
            menubar: true,      // 개발 7 — 필드 doc 참고
            limits: BTreeMap::new(), // normalize 가 평평한 칸으로 체인마다 채운다
            auto_learn_trusted: false,
        }
    }
}

impl Settings {
    /// 기존 사용자를 대신할 보수적 기본값 — 체인만 테스트넷.
    ///
    /// 신규 기본(`default()`)과 보수적 폴백을 갈라 둔다 — 신규 기본이 언젠가 메인넷이 돼도 «기본값으로 접는»
    /// 경로가 기존 사용자를 실돈 체인으로 옮기지 않게. **진짜 신규**(지갑도 설정도 없음)만 default() 를 받고,
    /// 아래 두 경우는 이걸 받는다 —
    /// ① 설정 파일이 **있는데 못 읽는** 경우(깨짐·권한): 표시·동작용 폴백. 저장은
    ///    read_settings_for_update 가 막는다.
    /// ② 설정 파일은 없는데 **지갑 파일이 있는** 경우(개발 31 이전 설치는 저장 버튼을
    ///    눌러야만 settings.json 이 생겼다): 테스트넷 시절 사용자다 — 이 경우는
    ///    reconcile 이 이 값을 저장해 테스트넷을 명시적으로 못박는다.
    /// 한도 둘을 「모름」으로 — 결제 쪽 `limits::parse_limit_*` 가 이 값을 보면 막는다.
    fn with_limits_unknown(mut self) -> Self {
        for slot in [
            &mut self.single_usdc,
            &mut self.daily_usdc,
        ] {
            *slot = crate::limits::LIMIT_UNKNOWN.into();
        }
        self.limits.clear(); // normalize 가 「모름」을 체인마다 복사한다
        self
    }

    /// 지금 평평한 칸의 한도 셋.
    fn flat_limits(&self) -> NetLimits {
        NetLimits {
            single_usdc: self.single_usdc.clone(),
            daily_usdc: self.daily_usdc.clone(),
            auto_approve_usdc: self.auto_approve_usdc.clone(),
        }
    }

    /// 읽은 뒤 한 번 — ① 체인별 한도에 빠진 체인은 평평한 칸 값으로 채운다(옛 파일 = 두 네트워크에 복사)
    /// ② 평평한 칸을 **활성 체인** 값으로 맞춘다 ③ 자동 잠금을 목록으로 접는다.
    /// 모르는 키(지원 안 하는 체인)는 남겨 둔다 — 지우면 그 체인을 아는 다른 빌드의 한도가 사라진다.
    pub(crate) fn normalize(mut self) -> Self {
        let flat = self.flat_limits();
        for id in policy::SUPPORTED_CHAIN_IDS {
            self.limits.entry(id.to_string()).or_insert_with(|| flat.clone());
        }
        if let Some(l) = self.limits.get(&self.chain_id.to_string()).cloned() {
            self.single_usdc = l.single_usdc;
            self.daily_usdc = l.daily_usdc;
            self.auto_approve_usdc = l.auto_approve_usdc;
        }
        self.auto_lock_mins = snap_lock_mins(&self.auto_lock_mins);
        self
    }

    fn conservative() -> Self {
        Settings {
            chain_id: policy::FALLBACK_CHAIN_ID,
            ..Settings::default()
        }
    }
}

/// 지갑 파일이 이미 있는가 — settings.json 이 없거나 필드가 비었을 때 "진짜 신규 설치"와
/// "기존 사용자"를 가른다. 암호화본(wallet.enc)과 옛 평문(wallet.json) 둘 다 본다.
/// (언어 기본값도 같은 질문을 쓴다 — i18n::init)
pub(crate) fn wallet_exists() -> bool {
    budu_dir()
        .map(|d| policy::wallet_exists_in(&d))
        .unwrap_or(false)
}

fn settings_path() -> Result<PathBuf, String> {
    Ok(budu_dir()?.join("settings.json"))
}

/// settings.json 을 세 갈래(없음/못 읽음/본문)로 읽는다. 홈을 못 정하면 「못 읽음」.
fn settings_file() -> SettingsFile {
    settings_path()
        .map(|p| SettingsFile::read(&p))
        .unwrap_or(SettingsFile::Unreadable)
}

/// 설정을 읽는다. 파일이 **없으면** 기본값(신규 = 데브넷), 있는데 **못 읽으면** 보수적
/// 기본값(데브넷). 판단만 떼어낸 `settings_for_read` 에 이유와 테스트가 있다.
pub(crate) fn read_settings() -> Settings {
    settings_for_read(&settings_file(), wallet_exists())
}

/// `read_settings` 의 판단만 떼어낸 순수 함수 (IO 없이 테스트하려고).
/// 없음 + 지갑 없음 = 진짜 신규 → 신규 기본(`NEW_INSTALL_CHAIN_ID`). 없음 + 지갑 있음 = 설정 파일이 없던 시절의 기존
/// 사용자 → 테스트넷 보수. 못 읽음·해석 안 됨 → 보수적 기본값.
///
/// 🔴 **체인만은 어느 갈래든 `policy::chain_id_for` 가 정한다** — chain.rs 의 `selected_chain_id`
/// (돈이 나가는 쪽)가 같은 파일을 읽는 바로 그 함수다. 정상 파일이면 Settings 가 읽은 값과 같고
/// (같은 필드를 같은 규칙으로 읽는다), 다른 필드 하나가 깨져 Settings 로는 못 읽는 파일이면 chain_id
/// 만은 그대로 살린다. 개발 52 실물 하네스에서 두 판정이 갈려 **화면은 「연습용 Base Sepolia」인데
/// 송금은 메인넷 실돈**이었던 것을 그때는 클로저로 이어 붙였고, 개발 56 에 판정 자체를 하나로 합쳤다
/// — 갈릴 길이 코드에 없다.
fn settings_for_read(file: &SettingsFile, wallet_exists: bool) -> Settings {
    let mut s = match file {
        SettingsFile::Missing if !wallet_exists => Settings::default(),
        SettingsFile::Missing => Settings::conservative(),
        // 있는데 못 읽음 — 사용자가 정한 한도를 모른다. 보수 기본(20)으로 채우면 낮춰 둔 한도가 **올라간다**.
        SettingsFile::Unreadable => Settings::conservative().with_limits_unknown(),
        SettingsFile::Text(text) => {
            serde_json::from_str(text).unwrap_or_else(|_| salvage_limits(text))
        }
    };
    s.chain_id = supported_or_fallback(policy::chain_id_for(file, || wallet_exists));
    // RPC 도 같다 — `effective_rpc`(잔액·송금이 붙는 쪽)와 MCP 가 같은 `policy::rpc_url_for`
    // 로 같은 파일을 읽는다. 한도 필드 하나가 깨진 파일에서 화면은 「공식」이라 그리는데 잔액은
    // 커스텀 RPC 에 붙거나(또는 그 반대) MCP 만 커스텀을 쓰는 상태를 없앤다.
    s.rpc_url = policy::rpc_url_for(file);
    s.normalize()
}

/// 통째로는 못 읽는 설정에서 **한도 넷만은 살린다**. 다른 필드 하나가 깨졌다고 보수 기본값으로
/// 가면 사용자가 1 USDC 로 낮춰 둔 일일 한도가 기본 20 으로 **올라가** 그만큼 더 나갈 수 있었다 — 체인·RPC 를 필드
/// 단위로 살리는 것과 같은 처방을 돈의 상한에도. JSON 자체가 깨져 아무것도 못 읽으면 보수 기본값 그대로다.
/// 자율 승인 한도는 살리지 않는다 — 보수 기본값이 「자율 꺼짐」이라 그쪽이 더 안전하다.
/// **못 살린 한도 칸은 「모름」(`LIMIT_UNKNOWN`)** 이다 — 72 까진 보수 기본 20 으로 채워, 1 로 낮춰 둔
/// 한도의 칸이 빠지거나(`null`·숫자) JSON 이 통째로 깨지면 한도가 **올라갔다**. 모르는 한도로는 결제하지 않는다
/// (`limits::parse_limit_*` 가 막고, 설정 화면에서 다시 저장하면 풀린다).
fn salvage_limits(text: &str) -> Settings {
    let mut s = Settings::conservative().with_limits_unknown();
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return s;
    };
    for (key, slot) in [
        ("single_usdc", &mut s.single_usdc),
        ("daily_usdc", &mut s.daily_usdc),
    ] {
        if let Some(x) = v.get(key).and_then(|x| x.as_str()) {
            *slot = x.to_string();
        }
    }
    // 체인별 한도도 같은 처방 — 있는 칸만 살리고 못 살린 칸은 「모름」, 자율 금액은 꺼짐(0).
    // 체인 항목 자체가 없으면 normalize 가 위 평평한 값으로 채운다.
    if let Some(map) = v.get("limits").and_then(|m| m.as_object()) {
        let unknown = || crate::limits::LIMIT_UNKNOWN.to_string();
        let str_of = |o: &serde_json::Value, k: &str| o.get(k).and_then(|x| x.as_str()).map(str::to_string);
        for (id, o) in map {
            if !o.is_object() {
                continue;
            }
            s.limits.insert(
                id.clone(),
                NetLimits {
                    single_usdc: str_of(o, "single_usdc").unwrap_or_else(unknown),
                    daily_usdc: str_of(o, "daily_usdc").unwrap_or_else(unknown),
                    auto_approve_usdc: DEFAULT_AUTO_APPROVE_USDC.into(),
                },
            );
        }
    }
    s
}

/// **덮어쓸 목적으로** 설정을 읽는다. 파일이 있는데 해석이 안 되면 `None`.
///
/// `read_settings` 는 어떤 실패든 기본값으로 삼킨다. 읽기만 할 땐 그게 맞다 — 설정을 못
/// 읽었다고 앱이 안 뜨면 더 손해다. 하지만 **읽은 값을 다시 저장하는 경로**에서는 그 관대함이
/// 그대로 데이터 유실이 된다: 해석 못 한 파일 위에 기본값을 쓰면 사용자의 한도·커스텀 RPC·
/// chain_id 가 한 번에 사라진다(메인넷 사용자가 조용히 테스트넷으로 돌아간다).
///
/// 개발 31 이전에는 settings.json 을 쓰는 곳이 `set_settings`(사용자가 저장 버튼을 누를 때)
/// 하나뿐이라 이 문제가 없었다. autostart::reconcile 이 **시작할 때마다** 읽고 쓰게 되면서
/// 생긴 위험이라, 저장 경로는 이쪽을 쓴다.
pub(crate) fn read_settings_for_update() -> Option<Settings> {
    settings_for_update(&settings_file(), wallet_exists())
}

/// `read_settings_for_update` 의 판단만 떼어낸 순수 함수 (IO 없이 테스트하려고).
/// 파일이 없을 때 지갑이 이미 있으면(설정 파일이 없던 시절의 기존 사용자) 보수적
/// 기본(테스트넷)을 돌려준다 — reconcile 이 이걸 저장해 테스트넷을 명시적으로 못박는다.
/// 진짜 신규(지갑도 없음)만 신규 기본(`default()`)을 받는다.
/// 있는데 못 읽음(권한·홈 못 정함)·해석 안 됨 → None: 덮어쓰지 않는다.
fn settings_for_update(file: &SettingsFile, wallet_exists: bool) -> Option<Settings> {
    match file {
        SettingsFile::Missing if !wallet_exists => Some(Settings::default().normalize()),
        SettingsFile::Missing => Some(Settings::conservative().normalize()),
        SettingsFile::Unreadable => None,
        SettingsFile::Text(text) => serde_json::from_str::<Settings>(text).ok().map(|mut s| {
            // 모르는 체인이면 RPC 도 같이 버린다 — 읽기 경로(`policy::rpc_url_for`)는 이미 버리는데,
            // 여기서 chain_id 만 접고 rpc_url 을 남기면 시작 때 autostart·언어 저장이 「데브넷 + 옛 체인 RPC」를
            // 파일에 박아 숨어 있던 RPC 가 되살아난다. 의도: 이 빌드가 모르는 chain_id 는 데브넷 + 공식 RPC 로 굳힌다.
            if chain_by_id(s.chain_id).is_none() {
                s.rpc_url.clear();
            }
            s.chain_id = supported_or_fallback(s.chain_id);
            s.normalize()
        }),
    }
}

/// 모르는 chain_id를 폴백(데브넷)으로.
/// 잔액·결제(active_chain)와 MCP 는 이미 그렇게 접는데, 화면·저장 경로가 원래 값을 들고 있으면 네트워크 버튼이 둘 다
/// 꺼진 채로 뜨고 한도만 바꿔 저장해도 `set_settings` 가 「지원하지 않는 체인」으로 **영원히** 거절했다(한도를 못 낮춘다).
fn supported_or_fallback(id: u64) -> u64 {
    chain_by_id(id).map(|c| c.chain_id).unwrap_or(policy::FALLBACK_CHAIN_ID)
}

/// 설정을 그대로 저장한다 (앱 내부용 — 사용자 입력 검증 없음).
///
/// `set_settings` 는 폼에서 온 값을 검증하는 경로라, 앱이 스스로 한 필드만 갱신할 때
/// 그걸 통과시킬 이유가 없다(그리고 통과시키면 옛 설정 파일의 어긋난 값 때문에
/// 자동 시작 기록 같은 게 저장에 실패한다). 검증이 필요한 값은 여기로 오지 않는다.
pub(crate) fn save_settings(settings: &Settings) -> Result<(), String> {
    write_json(settings_path()?, settings)
}

/// 설정 파일이 없고 지갑도 없으면(진짜 신규) 신규 기본을 저장한다. 그 밖엔 아무것도 안 한다.
pub(crate) fn pin_new_install() {
    if settings_file() == SettingsFile::Missing && !wallet_exists() {
        let _ = save_settings(&Settings::default().normalize());
    }
}

/// 잔액·송금·서명이 실제로 붙는 RPC. 판정은 `policy::rpc_url_for` + `policy::pick_rpc` — MCP
/// `effective_rpc` 와 **같은 함수**로 같은 settings.json 을 읽는다. 지정한 게 없으면 활성
/// 체인의 공식 RPC. GUI 에는 환경변수로 체인을 갈아타는 경로가 없어 `forced_other_chain` 은 늘 false.
pub(crate) fn effective_rpc() -> String {
    policy::pick_rpc(
        &policy::rpc_url_for(&settings_file()),
        false,
        active_chain().default_rpc,
    )
}

/// 에러·로그 문자열의 URL 을 `[RPC]` 로 가린다 — 정본은 `policy::redact_urls`(MCP·CLI 와 같은 함수).
pub(crate) use crate::policy::redact_urls;

/// 현재 한도 설정을 돌려준다 (없으면 기본값).
#[tauri::command]
pub(crate) fn get_settings() -> Settings {
    read_settings()
}

/// 설정 파일이 **있는데 못 읽고 있는가**. `read_settings` 는 그때 한도·옵션을 보수적
/// 기본값으로 조용히 접는다 — 사용자가 고른 값이 아니므로 화면에 알려야 한다(개발 51 하네스에서
/// 필드 하나 빠진 파일로 발견). 체인과 RPC 는 그 폴백을 타지 않는다(개발 56·57: `policy::chain_id_for`·
/// `rpc_url_for` 가 그 두 필드만 따로 읽어 화면·송금·MCP 가 같은 값을 본다) — 그래서 배너 문구는
/// 「체인·RPC 는 화면에 보이는 대로」 다. 동작은 안 바꾼다 — 알림만. 판정은
/// `read_settings_for_update` 와 같다: 파일 없음은 정상(false), 깨짐·권한·홈 못 정함이 true.
/// 저장이 막히는 조건과 같은 하나의 술어라 문구도 같은 상황을 가리킨다.
#[tauri::command]
pub(crate) fn settings_file_broken() -> bool {
    read_settings_for_update().is_none()
}

/// 화면 언어를 고른다. 앱 관리 필드라 "저장하고 닫기"와 무관하게 즉시 적용된다.
///
/// 저장에 실패하면 전역 언어도 **안 바꾼다** — 화면만 영어로 바뀌고 다음 실행에 한국어로
/// 돌아오면, 사용자는 자기가 뭘 잘못했는지 알 수 없다. 실패는 실패로 보이는 쪽이 낫다.
#[tauri::command]
pub(crate) fn set_lang(app: tauri::AppHandle, lang: String) -> Result<(), String> {
    let parsed = crate::i18n::parse(&lang);
    let mut s = read_settings_for_update().ok_or_else(|| {
        crate::i18n::ts!(
            ts!(
                "설정 파일을 읽지 못해서 저장하지 않았어요",
                "Couldn't read the settings file, so nothing was saved."
            ),
            "Couldn't read the settings file, so nothing was saved."
        )
        .to_string()
    })?;
    s.lang = Some(crate::i18n::code(parsed).to_string());
    save_settings(&s)?;
    crate::i18n::set(parsed);
    // 트레이 메뉴는 앱이 시작할 때 만들어져 있다 — 새 언어로 다시 붙인다.
    crate::tray::retitle(&app);
    Ok(())
}

/// 메뉴바 아이콘을 켜고 끈다. 저장이 먼저, 그다음 아이콘 — 저장에 실패하면 아이콘도 안 바꾼다
/// (다음 실행에 되돌아가면 사람은 자기가 뭘 잘못했는지 알 수 없다, set_lang 과 같은 이유).
#[tauri::command]
pub(crate) fn set_menubar(app: tauri::AppHandle, on: bool) -> Result<(), String> {
    let mut s = read_settings_for_update().ok_or_else(|| {
        ts!(
            "설정 파일을 읽지 못해서 저장하지 않았어요",
            "Couldn't read the settings file, so nothing was saved."
        )
        .to_string()
    })?;
    let before = s.menubar;
    s.menubar = on;
    save_settings(&s)?;
    // 아이콘을 못 바꿨으면 저장도 되돌린다 — 화면은 실패를 보이는데 다음 실행에 바뀐 값이 살아나면 안 된다.
    if let Err(e) = crate::tray::set_visible(&app, on) {
        s.menubar = before;
        let _ = save_settings(&s);
        return Err(e.to_string());
    }
    Ok(())
}

/// 메뉴바 아이콘을 두는가 — 설정 화면의 토글이 처음 그릴 때 묻는다.
#[tauri::command]
pub(crate) fn get_menubar() -> bool {
    read_settings().menubar
}

/// 현재 화면 언어 코드("ko"/"en"). 프론트가 첫 화면을 그리기 전에 물어본다 —
/// 설정에 값이 없으면 i18n::init 이 시작할 때 정한 **시스템 언어**가 나온다.
#[tauri::command]
pub(crate) fn get_lang() -> String {
    crate::i18n::code(crate::i18n::lang()).to_string()
}

/// 한도 설정을 저장한다. 모든 값이 십진수로 파싱되는지 검증 후 기록.
#[tauri::command]
pub(crate) fn set_settings(mut settings: Settings) -> Result<(), String> {
    // 체인을 먼저 검증한다(미지원 거부). 한도는 아래에서 **체인마다 그 체인의 decimals** 로 검증한다.
    chain_by_id(settings.chain_id).ok_or(ts!(
        "지원하지 않는 체인입니다",
        "That chain isn't supported"
    ))?;
    // 체인별 한도가 정본 — 빠진 체인은 채우고 평평한 칸을 들어온 체인 값으로 맞춘 뒤 **모든 체인**을 검증한다.
    // 음수 거부(parse_usdc_nonneg) — 옛 alloy 시절 음수 한도가 거대 U256 = "사실상 무제한"으로 둔갑해 가드레일이
    // 조용히 무력화되는 함정 차단. 0 은 정상(=무제한 의도). 모르는 체인 키는 normalize 가 남기지만 검증은 아는 체인만.
    settings = settings.normalize();
    for (id, l) in &settings.limits {
        let Some(c) = id.parse::<u64>().ok().and_then(chain_by_id) else {
            continue;
        };
        validate_limits(l, c.usdc_decimals)?;
    }
    settings.auto_lock_mins.trim().parse::<u64>().map_err(|_| {
        ts!(
            "자동 잠금(분)은 정수로 입력하세요",
            "Auto-lock minutes must be a whole number"
        )
        .to_string()
    })?;
    let rpc = settings.rpc_url.trim();
    if !(rpc.is_empty() || rpc.starts_with("http://") || rpc.starts_with("https://")) {
        return Err(ts!(
            "RPC 주소는 http(s):// 로 시작해야 합니다",
            "An RPC address has to start with http:// or https://"
        )
        .into());
    }
    // 앱 관리 필드는 클라이언트가 보낸 값을 버리고 디스크 값을 지킨다.
    // 프론트 Settings 타입엔 이것들이 없어서 폼 저장은 항상 기본값(None/true)을 실어 보낸다 —
    // 그대로 쓰면 "저장하고 닫기" 한 번에 자동 시작 희망값이 지워진다(=고치려던 버그의 재발).
    preserve_managed(&mut settings, &read_settings());
    save_settings(&settings)
}

/// 한도 셋을 검증한다 — 숫자·음수 아님.
fn validate_limits(l: &NetLimits, dec: u8) -> Result<(), String> {
    parse_usdc_nonneg(&l.single_usdc, dec).map_err(|_| {
        ts!(
            "단일 USDC 한도가 올바르지 않습니다 (음수 불가)",
            "The per-payment USDC limit isn't valid (no negatives)"
        )
        .to_string()
    })?;
    parse_usdc_nonneg(&l.daily_usdc, dec).map_err(|_| {
        ts!(
            "일일 USDC 한도가 올바르지 않습니다 (음수 불가)",
            "The daily USDC limit isn't valid (no negatives)"
        )
        .to_string()
    })?;
    parse_usdc_nonneg(&l.auto_approve_usdc, dec).map_err(|_| {
        ts!(
            "자율 결제 한도가 올바르지 않습니다 (음수 불가)",
            "The autopay limit isn't valid (no negatives)"
        )
        .to_string()
    })?;
    Ok(())
}

/// 설정 화면의 저장 — **이 창에서 바꾼 칸만** 지금 저장된 값 위에 얹는다.
///
/// 창이 둘이면 두 설정 화면이 같은 파일을 고친다. 폼 전체를 보내면(예전 `set_settings`) 손대지 않은 칸이
/// 그 창이 열릴 때의 옛 값으로 되돌아갔다 — 다른 창에서 끈 자율 결제가 되살아나는 식. 프론트에서 읽고 합쳐
/// 보내는 것도 안 된다(두 창이 같은 옛 값을 읽은 뒤 차례로 쓰면 같은 일이 난다). 합치기를 **여기 한 곳**에서
/// 하면 동기 커맨드라 메인 스레드에서 한 줄로 선다.
///
/// `base_chain_id` = 폼을 채울 때의 체인. 네트워크 칸을 안 바꿨는데 그 사이 다른 창이 체인을 바꿨으면 거절한다 —
/// 테스트넷을 보며 고친 한도가 메인넷에 조용히 얹히면 안 된다(메인넷 경고도 못 봤다).
/// 파일을 못 읽는 상태(깨짐)면 예전처럼 폼 전체를 쓴다 — 그게 깨진 설정을 푸는 길이다.
#[tauri::command]
pub(crate) fn save_settings_edit(
    form: Settings,
    changed: Vec<String>,
    base_chain_id: u64,
) -> Result<(), String> {
    let Some(cur) = read_settings_for_update() else {
        return set_settings(form);
    };
    let merged = merge_edit(&cur, &form, &changed, base_chain_id)?;
    set_settings(merged)
}

/// `save_settings_edit` 의 판단만 (IO 없이 테스트하려고).
fn merge_edit(
    cur: &Settings,
    form: &Settings,
    changed: &[String],
    base_chain_id: u64,
) -> Result<Settings, String> {
    if !changed.iter().any(|k| k == "chain_id") && cur.chain_id != base_chain_id {
        return Err(ts!(
            "설정을 연 뒤 다른 창에서 네트워크가 바뀌었어요. 저장하지 않았습니다 — 설정을 닫았다 다시 열어 주세요.",
            "The network was changed in another window after you opened Settings. Nothing was saved — close and reopen Settings."
        )
        .into());
    }
    let bad = || ts!("설정을 합치지 못했어요", "Couldn't merge the settings").to_string();
    let mut out = serde_json::to_value(cur).map_err(|_| bad())?;
    let src = serde_json::to_value(form).map_err(|_| bad())?;
    let (Some(o), Some(f)) = (out.as_object_mut(), src.as_object()) else {
        return Err(bad());
    };
    for k in changed {
        // 체인별 한도는 칸 하나씩 — `limits.<chain_id>.<칸>`. 통째로 얹으면 다른 창이 고친 다른 네트워크 한도가 되돌아간다.
        if let Some(rest) = k.strip_prefix("limits.") {
            let Some((id, field)) = rest.split_once('.') else { continue };
            if !["single_usdc", "daily_usdc", "auto_approve_usdc"].contains(&field) {
                continue;
            }
            let Some(v) = f.get("limits").and_then(|m| m.get(id)).and_then(|l| l.get(field)).cloned() else {
                continue;
            };
            let map = o.entry("limits").or_insert_with(|| serde_json::json!({}));
            let Some(map) = map.as_object_mut() else { continue };
            // 지금 파일에 그 체인이 없으면(있을 수 없지만) 폼의 그 체인 묶음을 통째로 바탕으로 쓴다.
            let entry = map
                .entry(id.to_string())
                .or_insert_with(|| f["limits"][id].clone());
            if let Some(e) = entry.as_object_mut() {
                e.insert(field.to_string(), v);
            }
            continue;
        }
        // 모르는 이름은 버린다 — 프론트가 엉뚱한 키를 보내도 파일에 새 칸이 생기지 않게.
        if o.contains_key(k) {
            if let Some(v) = f.get(k) {
                o.insert(k.clone(), v.clone());
            }
        }
    }
    serde_json::from_value(out).map_err(|_| bad())
}

/// 앱이 관리하는 필드(자동 시작 희망값·화면 언어·메뉴바)를 `from` 에서 가져온다.
fn preserve_managed(settings: &mut Settings, from: &Settings) {
    settings.autostart = from.autostart;
    settings.lang = from.lang.clone();
    settings.menubar = from.menubar;
}

#[cfg(test)]
mod tests;
