// 도움말 콘텐츠 — 인앱 도움말 화면(HelpScreen)의 한 벌.
// "사람 말로" 원칙: 전문용어 최소화, 처음 켠 사람이 바로 이해하게.
//
// 한국어 문장이 원본이다 — 영어는 그 한국어에 맞춰 썼다.
// 두 언어를 한 섹션 안에 나란히 둔다 — 문서 성격의 글이라 한쪽만 고치면 금세 어긋난다.
// 번역이 아니라 **같은 말을 그 언어로** 쓴다: 영어 쪽도 짧은 문장, 조용한 톤, 느낌표 없이.


import { t } from "./i18n";

/** 배포 문서·정확한 설정값이 있는 곳. 공개 저장소라 누구나 열린다. */
export const GITHUB_URL = "https://github.com/dinggi5/budu";

export type HelpSection = {
  id: string;
  title: string;
  /** 도움말 화면·투어에 그대로 들어가는 본문. */
  body: React.ReactNode;
};

const em = "text-[var(--color-ink-700)] dark:text-[#E8E5DD]";

export const HELP_SECTIONS: HelpSection[] = [
  {
    id: "what",
    title: t("부두가 뭐예요?", "What is Budu?"),
    body: t(
      <p>
        부두는 클로드, 코덱스와 같은 AI가 <b className={em}>부산항 무역 관련 항만 데이터</b>를 x402 기술을 통해 필요할 때마다
        자동 요청할 수 있게 해 주는 <b className={em}>AI 에이전트 전용 솔라나 블록체인 지갑</b>입니다.
      </p>,
      <p>
        Budu is <b className={em}>a Solana wallet made for AI agents</b>. It lets AIs like Claude and Codex request{" "}
        <b className={em}>Busan port and trade data</b> automatically, whenever they need it, using x402.
      </p>,
    ),
  },
  {
    id: "fund",
    title: t("AI한테 돈은 어떻게 주나요?", "How do I give the AI money?"),
    body: t(
      <div className="space-y-2.5">
        <p>
          AI가 지갑을 활용할 수 있게 지갑에 <b className={em}>USDC</b>를 채워야 합니다. <b className={em}>받기</b> 버튼을 누르면
          지갑 주소가 나옵니다.
        </p>
        <p>
          거래소나 다른 지갑에서 보낼 때는 네트워크를 반드시 <b className={em}>솔라나</b>로 골라야 합니다.
        </p>
      </div>,
      <div className="space-y-2.5">
        <p>
          Fill the wallet with <b className={em}>USDC</b> so the AI can use it. Press <b className={em}>Receive</b> to see the
          wallet address.
        </p>
        <p>
          When you send from an exchange or another wallet, always pick the <b className={em}>Solana</b> network.
        </p>
      </div>,
    ),
  },
  {
    id: "sol",
    title: t("결제에 솔라나가 필요한가요?", "Do I need SOL to pay?"),
    body: t(
      <p>
        결제에 <b className={em}>SOL은 없어도 됩니다.</b> 네트워크 수수료는 x402 판매자 쪽(이하 페이실리테이터)이 냅니다.
      </p>,
      <p>
        <b className={em}>You don't need SOL to pay.</b> The network fee is paid by the x402 seller's side (the facilitator).
      </p>,
    ),
  },
  {
    id: "flow",
    title: t("결제 과정은 어떻게 진행되나요?", "How does a payment go?"),
    body: t(
      <ol className="space-y-1.5 list-decimal pl-5 marker:text-[var(--color-ink-500)]">
        <li>AI가 데이터를 요청하고, 판매자가 x402 결제를 요청합니다.</li>
        <li>승인 창이 뜨고 금액과 판매자를 보여 줍니다.</li>
        <li>사용자가 비밀번호로 승인합니다. 자동 승인도 설정에서 켤 수 있습니다.</li>
        <li>결제 트랜잭션을 만들고 서명합니다.</li>
        <li>판매자가 정산하면 데이터가 AI에게 전달됩니다. 모든 결제 내역은 블록체인에 공개됩니다.</li>
      </ol>,
      <ol className="space-y-1.5 list-decimal pl-5 marker:text-[var(--color-ink-500)]">
        <li>The AI asks for data, and the seller asks for an x402 payment.</li>
        <li>The approval window opens and shows the amount and the seller.</li>
        <li>You approve with your password. You can also turn on automatic approval in Settings.</li>
        <li>The wallet builds the payment transaction and signs it.</li>
        <li>Once the seller settles, the data goes to the AI. Every payment is public on the blockchain.</li>
      </ol>,
    ),
  },
  {
    id: "uses",
    title: t("부두로 어떤 일을 할 수 있나요?", "What can I do with Budu?"),
    body: t(
      <ul className="space-y-2">
        <li>
          <b className={em}>입항 준비 보고서</b> — AI에게 «오슬로호 12월 18일 부산 입항 준비해»라고 하면 AI가 파고·조석·터미널
          대기 데이터를 사서 입항 창과 대기 예상을 정리하고 CSV로 만듭니다.
        </li>
        <li>
          <b className={em}>기상 확인</b> — 부산 인근 파고·풍속 최신 관측을 한 건당 0.01 USDC로 호출할 수 있습니다.
        </li>
        <li>
          <b className={em}>다른 x402 판매자</b> — 블록체인 생태계가 커져 x402 시장이 활성화되면 여러 판매자에게서 다양한 데이터를
          호출할 수 있습니다.
        </li>
      </ul>,
      <ul className="space-y-2">
        <li>
          <b className={em}>Port-call report</b> — tell the AI «prepare the Oslo's Busan arrival on December 18» and it buys
          wave, tide and terminal-wait data, lays out the arrival window and expected wait, and makes a CSV.
        </li>
        <li>
          <b className={em}>Weather check</b> — the latest wave height and wind near Busan, 0.01 USDC per call.
        </li>
        <li>
          <b className={em}>Other x402 sellers</b> — as the blockchain ecosystem grows and the x402 market takes off, you can
          call all kinds of data from many sellers.
        </li>
      </ul>,
    ),
  },
  {
    id: "cost",
    title: t("비용은 어느 정도 나가나요?", "How much does it cost?"),
    body: t(
      <div className="space-y-3">
        <p>데모 판매자 budu-port 가격(파고·조석 0.01, 혼잡 0.05 USDC) 기준 예시입니다. 판매자마다 가격이 다릅니다.</p>
        <table className="w-full num">
          <tbody className="[&_td]:py-1.5 [&_td:last-child]:text-right">
            <tr><td>입항 보고서 1건</td><td>0.07~0.3 USDC</td></tr>
            <tr><td>배 1척 5분마다 감시</td><td>하루 약 14 USDC</td></tr>
            <tr><td>20척 감시</td><td>하루 약 288 USDC</td></tr>
          </tbody>
        </table>
      </div>,
      <div className="space-y-3">
        <p>Examples at the demo seller budu-port's prices (waves and tides 0.01, congestion 0.05 USDC). Prices differ by seller.</p>
        <table className="w-full num">
          <tbody className="[&_td]:py-1.5 [&_td:last-child]:text-right">
            <tr><td>One port-call report</td><td>0.07–0.3 USDC</td></tr>
            <tr><td>Watching one ship every 5 min</td><td>about 14 USDC a day</td></tr>
            <tr><td>Watching 20 ships</td><td>about 288 USDC a day</td></tr>
          </tbody>
        </table>
      </div>,
    ),
  },
  {
    id: "safety",
    title: t("결제 보호 장치가 따로 있나요?", "Is there anything that protects payments?"),
    // 사장이 준 문장 그대로(개발 14). 사실 문제가 보이면 고치지 말고 묻는다.
    body: t(
      <ul className="space-y-2">
        <li>
          <b className={em}>한도</b> — 결제당 한도, 하루 한도 등을 설정할 수 있습니다.
        </li>
        <li>
          <b className={em}>긴급 잠금</b> — 헤더의 방패 버튼을 누르면 모든 결제를 즉시 막아 사용자를 보호합니다.
        </li>
        <li>
          <b className={em}>프로젝트 예산</b> — 프로젝트마다 쓸 수 있는 예산을 정해 둘 수 있습니다.
        </li>
        <li>
          <b className={em}>정산 확인</b> — 서명한 결제는 블록체인 내역으로 직접 확인할 수 있습니다.
        </li>
      </ul>,
      <ul className="space-y-2">
        <li>
          <b className={em}>Limits</b> — set a per-payment limit, a daily limit and more.
        </li>
        <li>
          <b className={em}>Emergency lock</b> — press the shield button in the header to block every payment at once and protect you.
        </li>
        <li>
          <b className={em}>Project budget</b> — set a budget for each project.
        </li>
        <li>
          <b className={em}>Settlement check</b> — every payment you sign can be checked yourself in the blockchain's history.
        </li>
      </ul>,
    ),
  },
  {
    id: "backup",
    title: t("시드 백업", "Seed backup"),
    body: t(
      <div className="space-y-2.5">
        <p>
          12개 단어로 된 <b className={em}>시드 키</b>로 자산의 소유를 증명할 수 있습니다. 단어를 잃어버리면 지갑을 되찾을 수
          없습니다.
        </p>
        <p>팬텀 등 다른 솔라나 지갑에 시드 키를 넣어도 같은 지갑에 접근할 수 있습니다.</p>
      </div>,
      <div className="space-y-2.5">
        <p>
          Your 12-word <b className={em}>seed</b> proves the funds are yours. Lose the words and the wallet can't be recovered.
        </p>
        <p>Put the seed into Phantom or another Solana wallet and you reach the same wallet.</p>
      </div>,
    ),
  },
];
