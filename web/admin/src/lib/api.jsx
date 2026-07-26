import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";

const apiBase = window.VPN_API_BASE_URL || "/api/v1";
const SESSION_KEY = "vpn-admin-session";

export class ApiError extends Error {
  constructor(message, status, code) {
    super(message);
    this.status = status;
    this.code = code;
  }
}

const toQuery = (params) => {
  const search = new URLSearchParams();
  Object.entries(params || {}).forEach(([key, value]) => {
    if (value === undefined || value === null || value === "") return;
    search.set(key, String(value));
  });
  const query = search.toString();
  return query ? `?${query}` : "";
};

export const readSession = () => {
  try {
    const saved = JSON.parse(sessionStorage.getItem(SESSION_KEY) || "null");
    return saved?.token && saved.expiresAt > Date.now() ? saved : null;
  } catch {
    return null;
  }
};

export const writeSession = (token, expiresAt) =>
  sessionStorage.setItem(SESSION_KEY, JSON.stringify({ token, expiresAt: new Date(expiresAt).getTime() }));

export const clearSession = () => sessionStorage.removeItem(SESSION_KEY);

export async function request(path, { method = "GET", body, params, token, signal } = {}) {
  const response = await fetch(`${apiBase}${path}${toQuery(params)}`, {
    method,
    signal,
    headers: {
      ...(body ? { "Content-Type": "application/json" } : {}),
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
    },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!response.ok) {
    const payload = await response.json().catch(() => ({}));
    throw new ApiError(payload.message || "Не удалось выполнить запрос", response.status, payload.code);
  }
  return response.status === 204 ? null : response.json();
}

const ApiContext = createContext(null);

export function ApiProvider({ token, onUnauthorized, children }) {
  const value = useMemo(
    () => ({
      token,
      call: async (path, options = {}) => {
        try {
          return await request(path, { ...options, token });
        } catch (error) {
          if (error instanceof ApiError && error.status === 401) onUnauthorized?.();
          throw error;
        }
      },
    }),
    [token, onUnauthorized],
  );
  return <ApiContext.Provider value={value}>{children}</ApiContext.Provider>;
}

export const useApi = () => {
  const context = useContext(ApiContext);
  if (!context) throw new Error("useApi requires ApiProvider");
  return context;
};

/**
 * Fetches a single endpoint and re-fetches whenever the serialized params change.
 * Keeps the previous payload visible while refreshing so tables do not flash empty.
 */
export function useQuery(path, { params, enabled = true, optional = false } = {}) {
  const { call } = useApi();
  const [state, setState] = useState({ data: null, error: "", loading: enabled });
  const [nonce, setNonce] = useState(0);
  const serialized = JSON.stringify(params ?? {});
  const mounted = useRef(true);

  useEffect(() => () => {
    mounted.current = false;
  }, []);

  useEffect(() => {
    if (!enabled) {
      setState({ data: null, error: "", loading: false });
      return undefined;
    }
    const controller = new AbortController();
    setState((prev) => ({ ...prev, loading: true, error: "" }));
    call(path, { params: JSON.parse(serialized), signal: controller.signal })
      .then((data) => {
        if (!controller.signal.aborted) setState({ data, error: "", loading: false });
      })
      .catch((error) => {
        if (controller.signal.aborted || error.name === "AbortError") return;
        if (optional && error.status === 404) {
          setState({ data: null, error: "", loading: false });
          return;
        }
        setState((prev) => ({ data: prev.data, error: error.message, loading: false }));
      });
    return () => controller.abort();
  }, [call, path, serialized, enabled, optional, nonce]);

  const reload = useCallback(() => setNonce((value) => value + 1), []);
  return { ...state, reload };
}

/** Runs several endpoints in parallel and exposes one combined loading/error state. */
export function useQueries(entries, { enabled = true } = {}) {
  const { call } = useApi();
  const [state, setState] = useState({ data: {}, error: "", loading: enabled });
  const [nonce, setNonce] = useState(0);
  const serialized = JSON.stringify(entries);

  useEffect(() => {
    if (!enabled) return undefined;
    const controller = new AbortController();
    const list = JSON.parse(serialized);
    setState((prev) => ({ ...prev, loading: true, error: "" }));
    Promise.all(
      list.map(async ([key, path, options = {}]) => {
        try {
          return [key, await call(path, { ...options, signal: controller.signal })];
        } catch (error) {
          if (options.optional && error.status === 404) return [key, null];
          throw error;
        }
      }),
    )
      .then((pairs) => {
        if (!controller.signal.aborted) {
          setState({ data: Object.fromEntries(pairs), error: "", loading: false });
        }
      })
      .catch((error) => {
        if (controller.signal.aborted || error.name === "AbortError") return;
        setState((prev) => ({ data: prev.data, error: error.message, loading: false }));
      });
    return () => controller.abort();
  }, [call, serialized, enabled, nonce]);

  const reload = useCallback(() => setNonce((value) => value + 1), []);
  return { ...state, reload };
}

/** Wraps a mutating request with pending state and a caller-supplied success message. */
export function useMutation() {
  const { call } = useApi();
  const [pending, setPending] = useState(false);
  const run = useCallback(
    async (path, { method = "POST", body } = {}) => {
      setPending(true);
      try {
        return await call(path, { method, body });
      } finally {
        setPending(false);
      }
    },
    [call],
  );
  return { run, pending };
}
