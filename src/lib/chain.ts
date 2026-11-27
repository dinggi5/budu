// 프론트 클러스터 설정 (EVM → 솔라나) — 표시 이름·익스플로러·기본 RPC 같은 "보여주기" 값 + 활성 클러스터 컨텍스트.
//
// src-tauri/src/chain.rs · budu-mcp/src/chain.rs 와 평행한 프론트 사본이다(세 레이어가 같은
// 체인을 각자 기술 — 공유 소스 없음, 프로젝트 정책). 체인 추가 시 세 곳을 함께 갱신.
//
// 활성 체인은 더 이상 컴파일 고정 상수가 아니라 settings.chain_id 로 런타임 선택한다.
// WalletScreen 이 ChainProvider 로 활성 ChainConfig 를 내려주고, 화면들은 useChain() 으로 읽는다.

import { createContext, useContext } from "react";

import { t } from "./i18n";

export interface ChainConfig {
  /** 클러스터 번호 — settings.chain_id 와 매칭 (103 = devnet, 101 = mainnet-beta; 출처는 shared/policy.rs 주석). */
  id: number;
  /** 사람용 표시 이름. 언어를 타는 값이라 `t()` 로 만든다. */
  name: string;
  /** 익스플로러 이름 (버튼 라벨용). */
  explorerName: string;
  /** 익스플로러 트랜잭션 URL — 앞(`explorerTx`) + 서명 + 뒤(`explorerTxSuffix`). 솔라나 익스플로러는
   *  데브넷을 `?cluster=devnet` 꼬리로 가른다 — 꼬리를 빼면 **메인넷에서 찾아 「없는 거래」** 로 뜬다. */
  explorerTx: string;
  explorerTxSuffix: string;
  /** 설정 화면의 "공식 RPC" 옵션 URL (빈 값 저장 시 백엔드가 이 값으로 폴백). */
  defaultRpc: string;
  /** "로그 안 남김" 표방 대체 공개 RPC (설정 프리셋용). **없는 클러스터는 undefined** — 확인 안 한 걸
   *  있는 척 채우면 프라이버시 약속이 거짓말이 되므로 프리셋에서 빼고 "공식 / 직접 입력" 둘만 보여준다. */
  publicNode?: string;
  /** 데브넷이면 true — 받기 화면의 Faucet 노출, 메인넷 경고 등에 쓴다. */
  testnet: boolean;
  /** **거래소·다른 지갑의 네트워크 목록에 뜨는 이름** (메인넷만). 틀리게 안내하면 사용자가 다른 네트워크로
   *  입금해 **자금을 잃는다**. 데브넷은 거래소에서 보낼 일이 없어 비워 둔다. */
  depositNetwork?: string;
  /** 테스트 USDC Faucet (데브넷 전용). */
  usdcFaucet?: string;
  /** 테스트 SOL Faucet (데브넷 전용). x402 결제엔 SOL 이 필요 없다 — 데모 판매자 계정을 만들 때만. */
  solFaucet?: string;
}

/** 솔라나 데브넷 (연습용). 기본 클러스터 — 신규 설치·깨진 설정의 폴백(백엔드 기준). */
export const SOLANA_DEVNET: ChainConfig = {
  id: 103,
  name: t("솔라나 데브넷", "Solana devnet"),
  explorerName: "Solana Explorer",
  explorerTx: "https://explorer.solana.com/tx/",
  explorerTxSuffix: "?cluster=devnet",
  defaultRpc: "https://api.devnet.solana.com",
  testnet: true,
  usdcFaucet: "https://faucet.circle.com",
  solFaucet: "https://faucet.solana.com",
};

/** 솔라나 메인넷 (mainnet-beta, 실제 자금). 공개 RPC 는 요청 제한이 빡빡하다 — 실사용은 RPC 지정. */
export const SOLANA_MAINNET: ChainConfig = {
  id: 101,
  name: t("솔라나 메인넷", "Solana mainnet"),
  explorerName: "Solana Explorer",
  explorerTx: "https://explorer.solana.com/tx/",
  explorerTxSuffix: "",
  defaultRpc: "https://api.mainnet-beta.solana.com",
  testnet: false,
  depositNetwork: "Solana",
};

/** 트랜잭션 서명 → 익스플로러 URL (클러스터 꼬리 포함). */
export function explorerTxUrl(chain: ChainConfig, sig: string): string {
  return chain.explorerTx + sig + chain.explorerTxSuffix;
}

/** 선택 가능한 클러스터 (설정 토글 순서). 대회 데모 단계라 데브넷이 왼쪽·기본. */
export const CHAINS: ChainConfig[] = [SOLANA_DEVNET, SOLANA_MAINNET];

/** chain_id → ChainConfig. 설정 로드 전(undefined)과 **모르는 id** 둘 다 데브넷 — 백엔드 active_chain()·MCP 가
 *  모르는 id 를 데브넷으로 정규화하므로, 여기서 다른 걸 그리면 잔액은 데브넷인데 라벨만 딴 클러스터가 된다. */
export function chainFromId(id: number | undefined): ChainConfig {
  if (id === undefined) return SOLANA_DEVNET;
  return CHAINS.find((c) => c.id === id) ?? SOLANA_DEVNET;
}

// 활성 체인 컨텍스트 — WalletScreen 이 settings.chain_id 로 파생해 Provider 로 내려준다.
const ChainContext = createContext<ChainConfig>(SOLANA_DEVNET);
export const ChainProvider = ChainContext.Provider;
export function useChain(): ChainConfig {
  return useContext(ChainContext);
}
