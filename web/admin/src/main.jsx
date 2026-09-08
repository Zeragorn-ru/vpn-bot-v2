import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";

const emptySettings = { brand_name: "VECTOR", mini_app_url: "", admin_url: "", subscription_url: "", support_url: "" };

async function api(path, options = {}, token = "") {
  const headers = { "Content-Type": "application/json", ...(options.headers || {}) };
  if (token) headers.Authorization = `Bearer ${token}`;
  const response = await fetch(path, { ...options, headers });
  const body = response.status === 204 ? null : await response.json().catch(() => null);
  if (!response.ok) throw new Error(body?.message || `Request failed (${response.status})`);
  return body;
}

function Setup({ onReady }) {
  const [form, setForm] = useState({ setup_token: "", login: "admin", password: "" });
  const [error, setError] = useState("");
  const submit = async (event) => {
    event.preventDefault();
    setError("");
    try { await api("/setup/owner", { method: "POST", body: JSON.stringify(form) }); onReady(); }
    catch (reason) { setError(reason.message); }
  };
  return <section className="card narrow"><p className="kicker">VECTOR / FIRST RUN</p><h1>Set the<br /><em>control plane.</em></h1><p>Use the one-time bootstrap token printed by the installer. It is deleted after the first owner is created.</p><form onSubmit={submit}><label>Setup token<input required value={form.setup_token} onChange={(event) => setForm({ ...form, setup_token: event.target.value })} /></label><label>Owner login<input required value={form.login} onChange={(event) => setForm({ ...form, login: event.target.value })} /></label><label>Password<input required minLength="12" type="password" value={form.password} onChange={(event) => setForm({ ...form, password: event.target.value })} /></label>{error && <div className="error">{error}</div>}<button>Create owner</button></form></section>;
}

function Login({ onLogin }) {
  const [form, setForm] = useState({ login: "", password: "" });
  const [error, setError] = useState("");
  const submit = async (event) => { event.preventDefault(); setError(""); try { const result = await api("/admin/auth/login", { method: "POST", body: JSON.stringify(form) }); sessionStorage.setItem("vector_admin_token", result.access_token); onLogin(result.access_token); } catch (reason) { setError(reason.message); } };
  return <section className="card narrow"><p className="kicker">VECTOR / ADMIN</p><h1>Welcome<br /><em>back.</em></h1><form onSubmit={submit}><label>Login<input required value={form.login} onChange={(event) => setForm({ ...form, login: event.target.value })} /></label><label>Password<input required type="password" value={form.password} onChange={(event) => setForm({ ...form, password: event.target.value })} /></label>{error && <div className="error">{error}</div>}<button>Sign in</button></form></section>;
}

function Console({ token }) {
  const [settings, setSettings] = useState(emptySettings);
  const [secret, setSecret] = useState({ key: "TELEGRAM_BOT_TOKEN", value: "" });
  const [message, setMessage] = useState("");
  useEffect(() => { api("/admin/settings/public", {}, token).then((value) => setSettings({ ...emptySettings, ...value })).catch((reason) => setMessage(reason.message)); }, [token]);
  const saveSettings = async (event) => { event.preventDefault(); try { await api("/admin/settings/public", { method: "PUT", body: JSON.stringify({ value: settings }) }, token); setMessage("Public settings saved"); } catch (reason) { setMessage(reason.message); } };
  const saveSecret = async (event) => { event.preventDefault(); try { await api(`/admin/secrets/${encodeURIComponent(secret.key)}`, { method: "PUT", body: JSON.stringify({ value: secret.value }) }, token); setSecret({ ...secret, value: "" }); setMessage("Encrypted secret saved"); } catch (reason) { setMessage(reason.message); } };
  const logout = async () => { await api("/admin/auth/logout", { method: "POST" }, token).catch(() => {}); sessionStorage.removeItem("vector_admin_token"); window.location.reload(); };
  return <main className="console"><header><div><p className="kicker">VECTOR / CONTROL PLANE</p><h1>Runtime,<br /><em>under control.</em></h1></div><button className="secondary" onClick={logout}>Sign out</button></header><div className="grid"><section className="card"><p className="kicker">PUBLIC ROUTING</p><h2>Where the product lives</h2><form onSubmit={saveSettings}>{Object.entries(settings).map(([key, value]) => <label key={key}>{key.replaceAll("_", " ")}<input value={value || ""} onChange={(event) => setSettings({ ...settings, [key]: event.target.value })} /></label>)}<button>Save settings</button></form></section><section className="card"><p className="kicker">ENCRYPTED SECRETS</p><h2>Write once, never echo</h2><p>Credentials are encrypted with the root key before PostgreSQL stores them. The API never returns their values.</p><form onSubmit={saveSecret}><label>Secret key<input value={secret.key} onChange={(event) => setSecret({ ...secret, key: event.target.value })} /></label><label>Secret value<input type="password" value={secret.value} onChange={(event) => setSecret({ ...secret, value: event.target.value })} /></label><button>Store encrypted secret</button></form></section></div>{message && <div className="notice">{message}</div>}</main>;
}

function App() {
  const [setup, setSetup] = useState(null);
  const [token, setToken] = useState(() => sessionStorage.getItem("vector_admin_token") || "");
  useEffect(() => { api("/setup/status").then((result) => setSetup(result.setup_required)).catch(() => setSetup(false)); }, []);
  if (setup === null) return <main className="loading">Checking readiness…</main>;
  if (setup) return <Setup onReady={() => setSetup(false)} />;
  if (!token) return <Login onLogin={setToken} />;
  return <Console token={token} />;
}

createRoot(document.getElementById("root")).render(<App />);
