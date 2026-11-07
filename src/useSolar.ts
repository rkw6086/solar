import { useCallback, useEffect, useRef, useState } from "react";
import { api, subscribe } from "./api";
import { emptyBootstrap, errorText } from "./types";
import type { Bootstrap } from "./types";

export function useSolar() {
  const [data, setData] = useState<Bootstrap>(emptyBootstrap);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const sequence = useRef(0);
  const refresh = useCallback(async () => {
    const current = ++sequence.current;
    try {
      const response = await api<Bootstrap>("get_bootstrap");
      if (current === sequence.current) {
        setData(response);
        setError("");
      }
    } catch (failure) {
      if (current === sequence.current) setError(errorText(failure));
    } finally {
      if (current === sequence.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    let disposed = false;
    let cleanup: (() => void) | undefined;
    void subscribe(() => void refresh())
      .then((unlisten) => {
        if (disposed) unlisten();
        else cleanup = unlisten;
      })
      .catch((failure) => setError(errorText(failure)));
    return () => {
      disposed = true;
      sequence.current += 1;
      cleanup?.();
    };
  }, [refresh]);

  return { data, loading, error, refresh };
}
