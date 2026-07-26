const apiBase = window.VPN_API_BASE_URL || "/api/v1";

export class ApiError extends Error {
  constructor(message, status) {
    super(message);
    this.status = status;
  }
}

export function createClient(getToken) {
  async function request(path, options = {}) {
    const token = getToken();
    const response = await fetch(`${apiBase}${path}`, {
      ...options,
      headers: {
        "Content-Type": "application/json",
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
        ...options.headers,
      },
    });
    if (!response.ok) {
      const body = await response.json().catch(() => ({}));
      throw new ApiError(body.message || `Request failed (${response.status})`, response.status);
    }
    return response.status === 204 ? null : response.json();
  }

  return {
    get: (path) => request(path),
    post: (path, body) => request(path, { method: "POST", body: JSON.stringify(body) }),
    put: (path, body) => request(path, { method: "PUT", body: JSON.stringify(body) }),
    del: (path) => request(path, { method: "DELETE" }),
    request,
  };
}

export async function authenticateTelegram(initData) {
  const response = await fetch(`${apiBase}/auth/telegram`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ init_data: initData }),
  });
  if (!response.ok) {
    const body = await response.json().catch(() => ({}));
    throw new ApiError(body.message || "Authentication failed", response.status);
  }
  return response.json();
}
