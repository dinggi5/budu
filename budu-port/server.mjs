// budu-port — 부산항 데이터를 AI 에이전트에게 x402(솔라나 데브넷 USDC)로 건당 파는 데모 판매자.
//
// 표준만 쓴다: 공식 미들웨어 `@x402/express` + 솔라나 `exact` 스킴 + 공개 페이실리테이터(x402.org, 데브넷 무료).
// 어떤 x402 솔라나 지갑으로도 살 수 있다 — 부두는 그중 «사람이 통제하는» 지갑일 뿐이다.
//
// 🔴 진짜/샘플 구분: 응답마다 `source` 가 있다 — "kma"(기상청 API허브 실관측) | "sample".
//    /wave 는 환경변수 KMA_AUTH_KEY 가 있으면 기상청 해양관측(sea_obs.php)을 실제로 부르고, 키가 없거나 실패하면
//    샘플을 내되 `live_error` 에 이유를 적는다(샘플을 진짜처럼 보이지 않게). /tide 는 국립해양조사원 API 의 응답 형식을
//    키로 확인하기 전까지 샘플이다. /congestion 은 공개 API 가 없어 늘 샘플.
//    부산항만공사·포트아이와 연동했다고 말하지 않는다.
//
// 실행: `npm run setup-seller`(한 번) → `KMA_AUTH_KEY=… npm start` → http://localhost:4021

import fs from "node:fs";
import express from "express";
import { paymentMiddleware, x402ResourceServer } from "@x402/express";
import { ExactSvmScheme } from "@x402/svm/exact/server";
import { HTTPFacilitatorClient } from "@x402/core/server";

const DEVNET = "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1";
const PORT = Number(process.env.PORT || 4021);
const FACILITATOR = process.env.FACILITATOR_URL || "https://x402.org/facilitator";

const sellerFile = new URL("./.seller-devnet.json", import.meta.url);
if (!fs.existsSync(sellerFile)) {
  console.error("판매자 주소가 없다 — 먼저 `npm run setup-seller` 를 실행하세요.");
  process.exit(1);
}
const PAY_TO = process.env.PAY_TO || JSON.parse(fs.readFileSync(sellerFile, "utf8")).address;

const resourceServer = new x402ResourceServer(new HTTPFacilitatorClient({ url: FACILITATOR })).register(
  DEVNET,
  new ExactSvmScheme(),
);

const offer = (price, description) => ({
  accepts: [{ scheme: "exact", price, network: DEVNET, payTo: PAY_TO, maxTimeoutSeconds: 120 }],
  description,
  mimeType: "application/json",
});

const KMA_KEY = (process.env.KMA_AUTH_KEY || "").trim();
// 부산 인근 두 지점: 거제도 부이(파고 있음) · 부산 조위관측소(파고 없음, 바람·수온·기압).
const WAVE_STATIONS = [
  { id: "22104", name: "거제도 부이" },
  { id: "158650", name: "부산" },
];
// 기상청 해양관측 명세(02-marine-obs/01-marine-integrated-obs.yml)의 출력 칸 순서.
const SEA_OBS_COLS = ["TP", "TM", "STN_ID", "STN_KO", "LON", "LAT", "WH", "WD", "WS", "WS_GST", "TW", "TA", "PA", "HM"];

/** 결측(-99 류)·숫자 아님 → null. 명세에 결측 표기가 없어 «-90 이하» 를 결측으로 본다. */
function num(v) {
  const n = Number(v);
  return Number.isFinite(n) && n > -90 ? n : null;
}

/** KST yyyymmddhhmi — sea_obs 는 이 시각에 가장 가까운 생산 시각(HH:00·HH:30)의 자료를 준다. */
function kstStamp(d = new Date()) {
  const k = new Date(d.getTime() + 9 * 3600 * 1000);
  return k.toISOString().slice(0, 16).replace(/[-T:]/g, "");
}

/**
 * sea_obs.php 텍스트 한 덩이를 지점별 행으로. 명세는 «고정길이/CSV» 라고만 해서 둘 다 받는다:
 * `#` 로 시작하는 줄은 머리말(칸 이름 줄이 있으면 그 순서를 쓴다), 나머지는 쉼표 또는 공백으로 가른다.
 * STN_KO 에 공백이 들어간 고정길이 줄은 칸 수가 어긋나므로 버린다(틀린 값을 내느니 결측).
 */
export function parseSeaObs(text) {
  let cols = SEA_OBS_COLS;
  const rows = [];
  for (const raw of String(text).split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;
    if (line.startsWith("#")) {
      const names = line.replace(/^#+/, "").trim().split(/[\s,]+/).filter((n) => n && n !== "=");
      if (names.includes("STN_ID") && names.includes("TM")) cols = names;
      continue;
    }
    const cells = line.includes(",") ? line.split(",").map((c) => c.trim()) : line.split(/\s+/);
    // 기상청 typ01 응답은 줄 끝에 빈 칸·`=` 꼬리가 붙기도 한다 — 칸 수를 세기 전에 떼어 낸다.
    while (cells.length && (cells.at(-1) === "" || cells.at(-1) === "=")) cells.pop();
    // 칸 수가 정확히 맞는 줄만 — 지점명에 공백이 든 고정길이 줄은 칸이 **넘쳐** 값이 한 칸씩 밀린다.
    if (cells.length !== cols.length) continue;
    const row = Object.fromEntries(cols.map((c, i) => [c, cells[i]]));
    rows.push(row);
  }
  return rows;
}

async function liveWave() {
  if (!KMA_KEY) throw new Error("KMA_AUTH_KEY 가 설정되지 않음");
  const out = [];
  for (const st of WAVE_STATIONS) {
    const url = new URL("https://apihub.kma.go.kr/api/typ01/url/sea_obs.php");
    url.search = new URLSearchParams({ tm: kstStamp(), stn: st.id, help: "0", authKey: KMA_KEY }).toString();
    const res = await fetch(url, { signal: AbortSignal.timeout(8000) });
    // 오류는 JSON {code, message} — 키 값이 섞이지 않게 상태 코드만 말한다.
    if (!res.ok) throw new Error(`기상청 API HTTP ${res.status}`);
    const row = parseSeaObs(await res.text()).find((r) => r.STN_ID === st.id);
    if (!row) throw new Error(`기상청 응답에 지점 ${st.id} 가 없음`);
    out.push({
      id: st.id,
      name: st.name,
      observed_at_kst: row.TM ?? null,
      wave_height_m: num(row.WH),
      wind_dir_deg: num(row.WD),
      wind_speed_ms: num(row.WS),
      water_temp_c: num(row.TW),
      pressure_hpa: num(row.PA),
    });
  }
  return out;
}

const app = express();

// 무료 카탈로그 — 에이전트가 무엇을 얼마에 살 수 있는지 먼저 본다.
app.get("/", (_req, res) => {
  res.json({
    name: "budu-port",
    about: "부산항 업무용 데이터 — x402(솔라나 데브넷 USDC)로 건당 결제",
    network: DEVNET,
    payTo: PAY_TO,
    endpoints: [
      { path: "/wave", price: "0.01 USDC", data: "부산 인근 파고·풍속·수온 최신 관측", source: KMA_KEY ? "kma (기상청 API허브 해양관측)" : "sample (기상청 키 미설정)" },
      { path: "/tide", price: "0.01 USDC", data: "부산항 만조·간조 예보", source: "sample (국립해양조사원 연동 준비 중)" },
      { path: "/congestion", price: "0.05 USDC", data: "부산항 터미널별 대기 선박·예상 대기", source: "sample — 공개 API 없음" },
    ],
  });
});

app.use(
  paymentMiddleware(
    {
      "GET /wave": offer("$0.01", KMA_KEY ? "부산 인근 파고·풍속 최신 관측 (기상청)" : "부산 인근 파고·풍속 최신 관측 (샘플)"),
      "GET /tide": offer("$0.01", "부산항 만조·간조 예보 (샘플)"),
      "GET /congestion": offer("$0.05", "부산항 터미널별 대기 선박·예상 대기 시간 (샘플)"),
    },
    resourceServer,
  ),
);

const now = () => new Date().toISOString();

app.get("/wave", async (_req, res) => {
  // 키가 없으면 처음부터 «샘플» 로 팔았다(402 설명·카탈로그) → 샘플을 준다.
  if (!KMA_KEY) {
    res.json(sampleWave("KMA_AUTH_KEY 가 설정되지 않음"));
    return;
  }
  // 키가 있으면 «기상청» 으로 팔았다 — 실패하면 샘플로 바꿔치지 않고 502 를 낸다. @x402/express 2.30.0 은 핸들러가
  // 400 이상으로 답하면 정산을 취소한다(handler_failed, dist/esm/index.mjs 실측) → 구매자 돈은 안 나간다.
  try {
    res.json({ source: "kma", provider: "기상청 API허브 해양관측(sea_obs)", fetched_at: now(), stations: await liveWave() });
  } catch (e) {
    res.status(502).json({ error: "기상청 관측을 가져오지 못했어요 — 결제는 정산되지 않습니다. 잠시 뒤 다시 시도하세요.", detail: String(e?.message ?? e) });
  }
});

function sampleWave(liveError) {
  return {
    source: "sample",
    live_error: liveError,
    observed_at: now(),
    stations: [
      { id: "22104", name: "거제도 부이", wave_height_m: 1.2, wind_speed_ms: 6.4 },
      { id: "158650", name: "부산", wave_height_m: null, wind_speed_ms: 5.1, note: "파고 관측 없는 지점" },
    ],
  };
}

app.get("/tide", (_req, res) => {
  res.json({
    source: "sample",
    port: "부산항",
    // 부산항 물때는 한국 날짜로 읽는다 — UTC 로 자르면 한국 오전 9시 전엔 어제 날짜가 나왔다.
    date: kstStamp().replace(/^(\d{4})(\d{2})(\d{2}).*/, "$1-$2-$3"),
    timezone: "Asia/Seoul",
    events: [
      { type: "high", time: "04:12", height_cm: 118 },
      { type: "low", time: "10:31", height_cm: 12 },
      { type: "high", time: "16:47", height_cm: 109 },
      { type: "low", time: "22:58", height_cm: 25 },
    ],
  });
});

app.get("/congestion", (_req, res) => {
  res.json({
    source: "sample",
    notice: "부산항 터미널 혼잡 데이터는 공개 API 가 없어 시연용 샘플입니다.",
    terminals: [
      { name: "신항 1부두", waiting_vessels: 3, est_wait_hours: 6 },
      { name: "신항 3부두", waiting_vessels: 1, est_wait_hours: 2 },
      { name: "북항 신선대", waiting_vessels: 4, est_wait_hours: 9 },
    ],
  });
});

if (process.env.BUDU_PORT_NO_LISTEN !== "1") {
  app.listen(PORT, () => {
    console.log(`budu-port: http://localhost:${PORT}  payTo=${PAY_TO}  facilitator=${FACILITATOR}  wave=${KMA_KEY ? "kma" : "sample"}`);
  });
}
