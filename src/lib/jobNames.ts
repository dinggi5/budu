// 작업 별명 — AI 가 붙인 작업 이름표는 예산 판정의 열쇠라 그대로 두고, 사람이 읽는 이름만 따로 단다(src-tauri/src/job_names.rs).
//
// 여러 화면(홈 패널·장부·판매자·승인 시트·팝오버 작업 바)이 같은 표를 본다 — 모듈 하나가 표를 들고 구독자에게 알린다.
// 바꾼 창은 `notifySync()` 로 다른 창에 알리고, 다른 창은 useSync 로 다시 읽는다.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { notifySync, useSync } from "./sync";

type Names = Record<string, string>;

let names: Names = {};
const subs = new Set<(n: Names) => void>();

function set(next: Names) {
  names = next;
  for (const f of subs) f(names);
}

export function reloadJobNames(): void {
  invoke<Names>("get_job_names")
    .then(set)
    .catch(() => {});
}

/** 별명을 바꾼다 — 빈 값이면 AI 가 붙인 이름표로 돌아간다. 실패는 부르는 쪽이 보여 준다. */
export async function renameJob(job: string, name: string): Promise<void> {
  const next = await invoke<Names>("rename_job", { job, name });
  set(next);
  notifySync();
}

/** 별명 표를 구독한다. `label(job)` = 보일 이름(별명이 없으면 이름표 그대로). */
export function useJobNames(): {
  label: (job: string) => string;
  renamed: (job: string) => boolean;
  /** 다른 작업의 별명이 이 이름표와 같다 — 승인 시트에서 «이어지는 작업» 으로 잘못 읽지 않게. */
  collides: (job: string) => boolean;
} {
  const [n, setN] = useState(names);
  useEffect(() => {
    subs.add(setN);
    reloadJobNames();
    return () => {
      subs.delete(setN);
    };
  }, []);
  useSync(reloadJobNames);
  return {
    label: (job: string) => n[job.trim()] || job,
    renamed: (job: string) => !!n[job.trim()],
    collides: (job: string) => Object.entries(n).some(([k, v]) => k !== job.trim() && v === job.trim()),
  };
}
