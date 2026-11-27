// parseSeaObs 단위 확인 — `node test-parse.mjs`. 실제 응답 모양은 키로 확인 전이라 명세의 두 꼴(CSV·공백)만 본다.
process.env.BUDU_PORT_NO_LISTEN = "1";
const { parseSeaObs } = await import("./server.mjs");
import assert from "node:assert/strict";
const csv = "#START7777\n# TP, TM, STN_ID, STN_KO, LON, LAT, WH, WD, WS, WS_GST, TW, TA, PA, HM\nB,202611211300,22104,거제도,128.9,34.77,1.3,40,7.2,9.1,17.8,14.2,1021.3,63\n#7777END\n";
const r = parseSeaObs(csv);
assert.equal(r.length, 1);
assert.equal(r[0].STN_ID, "22104");
assert.equal(r[0].WH, "1.3");
const ws = "N 202611211300 158650 부산 129.03 35.09 -99.0 210 5.1 6.0 16.2 13.0 1020.1 70\n";
const w = parseSeaObs(ws);
assert.equal(w[0].WH, "-99.0");
assert.equal(w[0].WS, "5.1");
assert.equal(parseSeaObs("N 2026 158650 부산 항 129\n").length, 0); // 칸 부족은 버린다
// 지점명에 공백 → 칸이 넘쳐 밀린 줄도 버린다(예전엔 WH 자리에 위도가 들어갔다)
assert.equal(parseSeaObs("N 202611211300 22104 거제 부이 128.9 34.77 1.3 40 7.2 9.1 17.8 14.2 1021.3 63\n").length, 0);
// 줄 끝 쉼표·= 꼬리는 칸으로 안 센다
assert.equal(parseSeaObs("B,202611211300,22104,거제도,128.9,34.77,1.3,40,7.2,9.1,17.8,14.2,1021.3,63,=\n")[0].WH, "1.3");
assert.equal(parseSeaObs("B,202611211300,22104,거제도,128.9,34.77,1.3,40,7.2,9.1,17.8,14.2,1021.3,63,\n")[0].HM, "63");
console.log("parseSeaObs ok");
