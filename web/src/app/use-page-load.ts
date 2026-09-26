import { useEffect, useRef, useState } from "react";
import { commitIfCurrent, createLatestGate, isStaleOrAborted } from "../api/index.ts";
import { useRefresh } from "./use-refresh.ts";

export interface PageLoad<T> {
  loading: boolean;
  data: T | undefined;
  error: unknown;
  reload: () => void;
}

export function usePageLoad<T>(
  load: (signal: AbortSignal) => Promise<T>,
  deps: readonly unknown[],
): PageLoad<T> {
  const { nonce } = useRefresh();
  const [tick, setTick] = useState(0);
  const [loading, setLoading] = useState(true);
  const [data, setData] = useState<T | undefined>(undefined);
  const [error, setError] = useState<unknown>(null);
  const loadRef = useRef(load);
  const gate = useRef(createLatestGate());
  loadRef.current = load;

  useEffect(() => {
    const controller = new AbortController();
    const id = gate.current.next();
    let active = true;
    setLoading(true);
    setError(null);
    setData(undefined);
    loadRef
      .current(controller.signal)
      .then((value) => {
        if (!active) {
          return;
        }
        commitIfCurrent(gate.current, id, controller.signal, value, (next) => {
          setData(next);
          setError(null);
        });
      })
      .catch((caught: unknown) => {
        if (
          !active ||
          controller.signal.aborted ||
          !gate.current.isCurrent(id) ||
          isStaleOrAborted(caught)
        ) {
          return;
        }
        setError(caught);
      })
      .finally(() => {
        if (active && gate.current.isCurrent(id)) {
          setLoading(false);
        }
      });
    return () => {
      active = false;
      controller.abort();
    };
    // 调用方把查询条件放进 deps。load 走 ref，避免每次渲染都重发。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nonce, tick, ...deps]);

  return {
    loading,
    data,
    error,
    reload: () => setTick((current) => current + 1),
  };
}
