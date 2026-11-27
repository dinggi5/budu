// 백엔드(Tauri 커맨드)와 주고받는 공유 타입. Rust 구조체와 필드가 일치해야 한다.

export type WalletInfo = { address: string };

/** 계정 하나 — 같은 시드의 HD 파생 인덱스(솔라나 m/44'/501'/n'/0', 팬텀과 같은 순서) + 주소 + 사람이 붙인 이름.
 *  label 이 빈 값이면 화면이 「계정 N」(N = index+1)으로 부른다. */
export type Account = { index: number; address: string; label: string };

export type WalletStatus = {
  state: "encrypted" | "legacy" | "none";
  /** **활성 계정**의 주소. 잔액·받기·보내기·내역이 이 주소 기준이다. */
  address: string | null;
  backed_up: boolean;
  /** 모든 계정(인덱스 순). encrypted 가 아니면 빈 배열. */
  accounts: Account[];
  /** 활성 계정의 파생 인덱스. */
  active: number;
  /** 평문 주소가 솔라나 꼴이 아닌 계정이 있다(EVM 시절 `0x…` 파일) → 옛 형식 화면에서 비번으로 고친다. */
  legacy_address: boolean;
};

/** 잔액 (솔라나). `sol` = 네이티브(수수료) 토큰 — x402 결제엔 필요 없다(수수료는 페이실리테이터).
 *  `usdc` = 결제에 쓰는 돈. 내 USDC 토큰 계정이 아직 없으면 "0.000000". */
export type Balances = { sol: string; usdc: string };

/** 네트워크 하나의 한도 셋. */
export type NetLimits = { single_usdc: string; daily_usdc: string; auto_approve_usdc: string };

export type Settings = {
  single_usdc: string;
  daily_usdc: string;
  /** 자율 결제 한도(USDC). 이 금액 이하 AI 결제는 세션 잠금 해제 시 비번 없이 자동 승인. "0"=꺼짐. */
  auto_approve_usdc: string;
  /** 세션 자동 잠금 유휴 시간(분). "0"=유휴 잠금 안 함. */
  auto_lock_mins: string;
  /** 잔액 조회·송금에 쓸 RPC 엔드포인트. 빈 값이면 공식 RPC. */
  rpc_url: string;
  /** 자리비움 자동 잠금: 창 포커스 잃으면 세션 즉시 잠금. */
  lock_on_blur: boolean;
  /** 자율 결제 알림: 비번 없이 자동 승인된 결제를 OS 알림으로 사후 통지. */
  notify_auto: boolean;
  /** 알림에서 금액 숨기기 — 켜면 알림 제목이 금액 없이 「자율 결제」로만 뜬다. */
  notify_hide_amount: boolean;
  /** 자율 결제는 화이트리스트(비번으로 승인한 적 있는 주소)만. */
  auto_trusted_only: boolean;
  /** 활성 클러스터 번호 — 103=솔라나 데브넷 / 101=솔라나 메인넷.
   *  클러스터별로 사용액·내역·신뢰목록·한도가 분리된다. 목록은 lib/chain.ts 의 CHAINS 가 정본. */
  chain_id: number;
  /** 체인별 한도(키 = chain_id 문자열) — 한도의 정본. 위 평평한 세 칸은 활성 체인 값을 비춘 보기 칸. */
  limits: Record<string, NetLimits>;
  /** 비번 승인한 x402 받는 주소를 화이트리스트에 저절로 넣을까. 기본 꺼짐. */
  auto_learn_trusted: boolean;
};

/** 백엔드가 찾은 업데이트. 사람이 설치를 누르기 전에 보는 값 = 판단 근거 전부. */
export type UpdateInfo = {
  version: string;
  current_version: string;
  notes: string | null;
  date: string | null;
};

/** `update://progress` 이벤트 payload. total 은 서버가 크기를 안 알려주면 null. */
export type UpdateProgress = {
  downloaded: number;
  total: number | null;
};

/** 자율 결제 세션 상태 (메모리의 잠금 해제 키). */
export type SessionStatus = {
  unlocked: boolean;
  remaining_secs: number;
  auto_limit: string;
};

export type SpendView = { usdc: string };

export type HistoryEntry = {
  ts: number;
  token: string;
  to: string;
  amount: string;
  status: string; // "signed"(정산 대기)|"settled"(정산됨)|"settle_failed"|"blocked"|"failed" + 정산 확정의 "expired"(블록해시 만료·안 나감)|"reverted"(체인에서 실패·안 나감)
  /** signed·settled = 우리 서명(base58), blocked·failed = 사유. */
  detail: string;
  /** x402 정산 tx(판매자가 전한 페이실리테이터 서명, base58). 정산 전엔 빈 값. */
  settle_tx?: string;
  /** x402 결제 대상 URL(표시용). 옛 기록은 없음. */
  resource?: string;
  /** 작업 이름표. 같은 값끼리 묶이고 영수증 CSV 가 작업 단위로 나간다. */
  job?: string;
  /** 체인에서 우리 서명을 확인했다 — settled 면 «체인 확인», expired·reverted 면 확정. */
  checked?: boolean;
  /** 돈은 나갔거나 나갔을 수 있는데 응답 내용을 못 받았다. */
  undelivered?: boolean;
  /** MCP 가 정산된 응답의 내용을 끝까지 받았다고 알렸다(긍정 신호). 없으면 «모름». */
  delivered?: boolean;
};

/** 열린 작업 예산(백엔드 `job::JobView`) — 사람이 승인 창에서 연 것. */
export type JobView = {
  job: string;
  spent: string;
  budget: string;
  left: string;
  secs_left: number;
  sellers: number;
  /** 사람이 승인한 판매자 주소(payTo) — 판매자 화면이 «이 작업에선 창 없이» 를 표시한다. */
  seller_addrs: string[];
  chain_id: number;
  /** 예산을 연 계정 주소. */
  from: string;
};

/** 대조 결과 하나 — 판정이 아니라 비교의 결과다.
 *  wallet: match(같음) | differs(다름) | unset(등록 지갑 없음) | unknown(비교 불가)
 *  domain: match | differs | unknown */
export type PaymentRequest = {
  id: string;
  token: string;
  to: string;
  amount: string;
  memo: string;
  created: number;
  /** "x402-svm"(솔라나 부분 서명)만 받는다. 그 밖·없음은 백엔드가 거절한다. */
  kind?: string;
  /** x402일 때 결제 대상 리소스 URL. */
  resource?: string;
  /** 요청 생성 시점의 체인 ID. 승인 시 현재 활성 체인과 다르면 백엔드가 거부. */
  chain_id?: number;
  /** 요청 생성 시점의 활성 계정 — 파생 인덱스 + 주소. 승인 시 지금 계정과 다르면
   *  백엔드가 거부하고, 같으면 승인 전체를 그 계정으로 고정한다. from 이 비면 옛 요청(검사 없음). */
  account?: number;
  from?: string;
  /** x402 솔라나 결제 조건 — 승인 창이 수수료 지불자·온체인 메모를 보여 준다. */
  svm?: SvmTerms;
  /** 작업 이름표 — 있으면 승인 창이 «이 작업 전체를 N USDC 까지» 를 고르게 한다. */
  job?: string;
};

/** MCP 가 402 에서 고른 솔라나 exact 조건 — 백엔드 `policy::SvmTerms` 와 같은 모양. */
export type SvmTerms = {
  network: string;
  asset: string;
  pay_to: string;
  /** base unit 정수 문자열. */
  amount: string;
  /** 수수료 지불자(페이실리테이터). */
  fee_payer: string;
  /** 판매자가 정한 온체인 메모. 없으면 지갑이 난수를 넣는다. */
  memo?: string;
};

/** 승인 창의 한도 상황 — 빈 값 = 그 한도 없음. */
export type LimitRoom = { single: string; daily: string; spent: string; left_after: string };

export type AgentStatus = { connected: boolean; client: string };

/** AI 연결 화면 감지 상태 — 전부 로컬 파일 감지, 최종 진실은 AgentStatus. */
export type ConnectStatus = {
  /** 찾아낸 codex CLI 경로 — 있으면 «연결» 버튼, 없으면 명령 복사. */
  codex_path: string | null;
  codex_registered: boolean;
  codex_registered_other: boolean;
  desktop_installed: boolean;
  desktop_ext_installed: boolean;
  /** 확장은 깔려 있는데 Claude 데스크톱 설정에서 꺼 둠 — 「준비됨」이 아니다. */
  desktop_ext_disabled: boolean;
  cli_path: string | null;
  /** budu 등록이 있고 command 가 이 빌드의 budu-mcp 경로와 일치. */
  cli_registered: boolean;
  /** budu 등록은 있는데 다른 경로를 가리킴(옛 설치 등) — 재등록 안내 대상. */
  cli_registered_other: boolean;
  /** 등록·안내용 budu-mcp 경로. 임시 위치 실행 중이면 설치본으로 해석된 값. */
  mcp_path: string | null;
  /** 임시 위치(App Translocation·DMG)에서 실행 중 — mcp_path 까지 없으면 이전 안내. */
  temp_location: boolean;
  /** 지금 실행 중인 이 앱의 버전. */
  app_version: string;
  /**
   * 임시 위치 실행이라 **설치본** 경로를 등록하게 되는데 그 설치본이 이 앱과 다른
   * 버전일 때, 그 설치본 버전. AI 에 붙을 budu-mcp 는 설치본 안의 것이라 화면과
   * 실물이 갈린다. 같거나 못 읽으면 null.
   */
  installed_version_mismatch: string | null;
};

/** connect_claude_code 가 실패할 때 주는 모양. */
export type ConnectError = {
  message: string;
  /**
   * 옛 budu 등록을 되살리는 `claude mcp add-json …` 명령. 화면이 안내하는 수동
   * 재등록은 `remove; add` 라서, add 가 또 실패하면 원복해 둔 등록까지 지운다 —
   * 그때 되돌릴 손잡이. 지우기 전에 옛 항목이 있었을 때만 채워진다.
   */
  restore_command: string | null;
};
