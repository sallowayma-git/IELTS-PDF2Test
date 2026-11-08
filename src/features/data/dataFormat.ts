import { useCallback, useEffect, useState } from "react";
import { toUserFacingError } from "../../utils/userFacingError";

/** 难题判定门槛：单题有效样本 N 不足时只列出，不下结论。 */
export const HARD_MIN_N = 30;
/** 难题：N >= 30 且正确率低于此值。 */
export const HARD_ACCURACY_THRESHOLD = 0.5;

export function pct(value: number | null | undefined): string {
  return value == null || Number.isNaN(value) ? "—" : `${Math.round(value * 100)}%`;
}

export function num(value: number | null | undefined, digits = 1): string {
  if (value == null || Number.isNaN(value)) return "—";
  return Number.isInteger(value) ? String(value) : value.toFixed(digits);
}

export function nText(n: number): string {
  return `N=${n}`;
}

export function secText(sec: number | null | undefined): string {
  if (sec == null) return "—";
  const minutes = Math.floor(sec / 60);
  const rest = sec % 60;
  return minutes > 0 ? `${minutes}分${rest}秒` : `${rest}秒`;
}

export function msText(ms: number | null | undefined): string {
  if (ms == null) return "—";
  return `${(ms / 1000).toFixed(1)}s`;
}

export function timeText(iso: string | null | undefined): string {
  if (!iso) return "—";
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? iso : date.toLocaleString("zh-CN", { hour12: false });
}

export function paperText(paperId: string): string {
  return paperId ? paperId : "旧套题";
}

/** 单题结论文案：N 不足只说明样本量，不给难题结论。 */
export function questionVerdict(question: { n: number; accuracy: number | null }): string {
  if (question.n < HARD_MIN_N) return `样本不足（${nText(question.n)}，<${HARD_MIN_N}），仅列出，不下难题结论`;
  if (question.accuracy != null && question.accuracy < HARD_ACCURACY_THRESHOLD) return "难题";
  return "未达难题标准";
}

export function errorText(error: unknown, fallback: string): string {
  return toUserFacingError(error, fallback).userMessage;
}

export interface LoadState<T> {
  data: T | undefined;
  error: string | undefined;
  loading: boolean;
  reload: () => void;
}

/** 异步读取一次（deps 变化或 reload 时重读）；`enabled=false` 时不发请求。 */
export function useLoad<T>(
  load: () => Promise<T>,
  deps: ReadonlyArray<unknown>,
  fallbackError: string,
  enabled = true
): LoadState<T> {
  const [data, setData] = useState<T | undefined>(undefined);
  const [error, setError] = useState<string | undefined>(undefined);
  const [loading, setLoading] = useState(enabled);
  const [tick, setTick] = useState(0);
  const reload = useCallback(() => setTick((value) => value + 1), []);

  useEffect(() => {
    if (!enabled) {
      setLoading(false);
      return;
    }
    let alive = true;
    setLoading(true);
    setError(undefined);
    load()
      .then((result) => {
        if (alive) setData(result);
      })
      .catch((caught: unknown) => {
        if (alive) setError(errorText(caught, fallbackError));
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, tick, enabled]);

  return { data, error, loading, reload };
}
