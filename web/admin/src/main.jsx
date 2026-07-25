import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import "../styles.css";

const apiBase = window.VPN_API_BASE_URL || "/api/v1";
const preview = new URLSearchParams(location.search).get("preview") === "1";
const providerNames = { crypto_pay: "Crypto Pay", anore: "Anore", telegram_stars: "Telegram Stars" };
const money = (value, currency = "RUB") => new Intl.NumberFormat("ru-RU", { style: "currency", currency, maximumFractionDigits: currency === "XTR" ? 0 : 2 }).format(value / (currency === "XTR" ? 1 : 100));
const date = (value) => value ? new Intl.DateTimeFormat("en-GB", { day: "2-digit", month: "short", year: "numeric" }).format(new Date(value)) : "-";
const label = (value) => String(value || "").replaceAll("_", " ");

function App() {
  const [token, setToken] = useState(null);
  const [data, setData] = useState(null);
  const [tab, setTab] = useState("overview");
  const [setupRequired, setSetupRequired] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");

  const request = async (path, options = {}, session = token) => {
    const response = await fetch(`${apiBase}${path}`, { ...options, headers: { "Content-Type": "application/json", ...(session ? { Authorization: `Bearer ${session}` } : {}), ...options.headers } });
    if (!response.ok) throw new Error((await response.json().catch(() => ({}))).message || "Request failed");
    return response.status === 204 ? null : response.json();
  };
  const load = async (session) => {
    const get = (path) => request(path, {}, session);
    const [dashboard, settings, referralSettings, providers, publicSettings, runtimeSettings, telegramTransport, tariffs, channels, promos, users, subscriptions, invoices, audit] = await Promise.all([
      get("/admin/dashboard"), get("/admin/trial-settings"), get("/admin/referral-settings"), get("/admin/payment-providers"), get("/admin/public-settings"), get("/admin/runtime-settings"), get("/admin/telegram-transport"), get("/admin/tariffs"), get("/admin/required-channels"), get("/admin/promos"), get("/admin/users?limit=25&offset=0"), get("/admin/subscriptions?limit=25&offset=0"), get("/admin/invoices?limit=25&offset=0"), get("/admin/audit?limit=25&offset=0")
    ]);
    setData({ dashboard, settings, referralSettings, providers, publicSettings, runtimeSettings, telegramTransport, tariffs, channels, promos, users, subscriptions, invoices, audit });
  };
  const bootstrap = async () => {
    setLoading(true); setError("");
    try {
      if (preview) { setToken("preview"); setData(previewData()); return; }
      const saved = JSON.parse(sessionStorage.getItem("vpn-admin-session") || "null");
      if (saved?.token && saved.expiresAt > Date.now()) { setToken(saved.token); await load(saved.token); return; }
      const status = await request("/admin/setup-status"); setSetupRequired(status.setup_required);
    } catch (cause) { setError(cause.message); }
    finally { setLoading(false); }
  };
  useEffect(() => { bootstrap(); }, []);
  const authenticate = async ({ login, password }) => {
    setLoading(true); setError("");
    try {
      const auth = await request(setupRequired ? "/admin/setup" : "/admin/login", { method: "POST", body: JSON.stringify({ login, password }) }, null);
      sessionStorage.setItem("vpn-admin-session", JSON.stringify({ token: auth.access_token, expiresAt: new Date(auth.expires_at).getTime() }));
      setToken(auth.access_token); await load(auth.access_token);
    } catch (cause) { setError(cause.message); }
    finally { setLoading(false); }
  };
  const save = async (path, body, message) => {
    try {
      if (!preview) await request(path, { method: "PUT", body: JSON.stringify(body) });
      setNotice(preview ? "Preview change applied locally." : message);
      if (!preview) await load(token);
      else setData((current) => ({ ...current, ...(path === "/admin/public-settings" ? { publicSettings: body } : {}), ...(path === "/admin/runtime-settings" ? { runtimeSettings: body } : {}), ...(path === "/admin/telegram-transport" ? { telegramTransport: body } : {}) }));
    } catch (cause) { setNotice(cause.message); }
  };
  if (loading) return <State title="Loading control plane" />;
  if (!token && !preview) return <AdminLogin setupRequired={setupRequired} error={error} onSubmit={authenticate} />;
  if (error) return <State title="Access unavailable" detail={error} action={bootstrap} />;
  const pages = {
    overview: <Overview data={data.dashboard} />,
    users: <Users data={data.users} />,
    payments: <Invoices data={data.invoices} />,
    subscriptions: <Subscriptions data={data.subscriptions} />,
    tariffs: <Catalog title="Tariffs" items={data.tariffs} />,
    promos: <Catalog title="Promos" items={data.promos} />,
    channels: <Catalog title="Required channels" items={data.channels} />,
    integrations: <Integrations items={data.providers} />,
    policies: <Policies settings={data.settings} referrals={data.referralSettings} onSave={save} />,
    runtime: <Runtime data={data} onSave={save} />,
    audit: <Audit data={data.audit} />,
    system: <System />,
  };
  return <><header className="topbar"><button className="brand" onClick={() => setTab("overview")}><b /> VPN//OPS <span>control plane</span></button><div className="top-actions"><span className="live"><i />{preview ? "PREVIEW DATA" : "SESSION VERIFIED"}</span><button className="text-button" onClick={() => { sessionStorage.removeItem("vpn-admin-session"); setToken(null); setData(null); bootstrap(); }}>Log out</button></div></header><div className="admin-shell"><Nav tab={tab} setTab={setTab} /><main className="content">{notice && <button className="notice" onClick={() => setNotice("")}>{notice}<span>×</span></button>}{pages[tab]}</main></div></>;
}

function AdminLogin({ setupRequired, error, onSubmit }) {
  const [login, setLogin] = useState(""); const [password, setPassword] = useState(""); const [confirm, setConfirm] = useState(""); const [localError, setLocalError] = useState("");
  const submit = (event) => { event.preventDefault(); if (setupRequired && password !== confirm) { setLocalError("Passwords do not match."); return; } onSubmit({ login, password }); };
  return <main className="login-shell"><section className="login-card"><p className="eyebrow">VECTOR / CONTROL PLANE</p><h1>{setupRequired ? "Create root admin" : "Administrator login"}</h1><p className="helper">{setupRequired ? "This local first-run account owns all future administrator access." : "Use your administrator login and password."}</p><form onSubmit={submit}><label>Login<input value={login} autoComplete="username" minLength="3" onChange={(event) => setLogin(event.target.value)} required /></label><label>Password<input value={password} type="password" autoComplete={setupRequired ? "new-password" : "current-password"} minLength="12" onChange={(event) => setPassword(event.target.value)} required /></label>{setupRequired && <label>Confirm password<input value={confirm} type="password" autoComplete="new-password" minLength="12" onChange={(event) => setConfirm(event.target.value)} required /></label>}<button type="submit">{setupRequired ? "Create root administrator" : "Sign in"}</button></form>{(localError || error) && <p className="login-error">{localError || error}</p>}<small>Initial setup is available only through the loopback admin port.</small></section></main>;
}
function Nav({ tab, setTab }) { const groups = [["MAIN", [["overview", "Overview"], ["users", "Clients"]]], ["OPERATIONS", [["payments", "Payments"], ["subscriptions", "Subscriptions"], ["tariffs", "Tariffs"], ["promos", "Promos"]]], ["ACCESS", [["channels", "Required channels"], ["integrations", "Integrations"]]], ["SETTINGS", [["policies", "Policies"], ["runtime", "Runtime"], ["audit", "Audit log"], ["system", "System"]]]]; return <aside className="sidebar">{groups.map(([name, items]) => <section className="nav-group" key={name}><small>{name}</small>{items.map(([id, title]) => <button className={`nav-item ${tab === id ? "active" : ""}`} onClick={() => setTab(id)} key={id}>{title}</button>)}</section>)}<div className="sidebar-foot">ADMIN / V2<br />AUDITED OPERATIONS</div></aside>; }
function Heading({ kicker, title, description, children }) { return <section className="section-head"><div><p>{kicker}</p><h1>{title}</h1><span>{description}</span></div>{children}</section>; }
function State({ title, detail, action }) { return <main className="state"><i /><h1>{title}</h1>{detail && <p>{detail}</p>}{action && <button onClick={action}>Retry</button>}</main>; }
function Overview({ data }) { return <><Heading kicker="MAIN / OVERVIEW" title="Control room" description="Commercial operations and delivery at a glance." /><section className="metric-grid"><Metric label="Clients" value={data.registered_users} /><Metric label="Active access" value={data.active_subscriptions} /><Metric label="Paid invoices" value={data.paid_invoices} /><Metric label="Revenue" value={money(data.paid_revenue_rub_minor)} /></section><section className="split-grid"><article className="panel"><div className="panel-title"><div><p>OPERATOR QUEUE</p><h2>Needs attention</h2></div></div><Row title="Pending invoices" value={data.pending_invoices} /><Row title="Provisioning pending" value={data.provisioning_pending_subscriptions} /></article><article className="panel"><div className="panel-title"><div><p>CONTROL PLANE</p><h2>Release boundary</h2></div></div><p className="helper">Public hostnames and TLS stay under manual host nginx administration. This panel stores application settings only.</p></article></section></>; }
function Metric({ label, value }) { return <article className="metric"><span>{label}</span><strong>{value}</strong><small>Current aggregate</small></article>; }
function Row({ title, value }) { return <div className="signal-row"><span>{title}</span><b>{value}</b></div>; }
function Users({ data }) { return <><Heading kicker="MAIN / CLIENTS" title="Clients" description="Registered customers and wallet state." /><section className="card-grid">{data.items.map((item) => <article className="user-card" key={item.id}><div><span className="avatar">{(item.first_name || "?")[0]}</span><span><strong>{item.username ? `@${item.username}` : item.first_name}</strong><small>{item.telegram_user_id}</small></span></div><b className="badge">{item.language_code}</b><div className="card-stats"><span><small>Wallet</small><b>{money(item.balance_minor, item.currency_code)}</b></span><span><small>Created</small><b>{date(item.created_at)}</b></span></div></article>)}</section><Pager data={data} /></>; }
function Invoices({ data }) { return <><Heading kicker="OPERATIONS / PAYMENTS" title="Payments" description="Provider states and invoice lifecycle." /><DataList data={data} render={(item) => <><div><p>{providerNames[item.provider] || label(item.provider)}</p><h2>{money(item.amount_minor, item.currency_code)}</h2><small>{label(item.purpose)} · {date(item.created_at)}</small></div><b className={`badge ${item.status === "paid" ? "ok" : "warn"}`}>{label(item.status)}</b></>} /></>; }
function Subscriptions({ data }) { return <><Heading kicker="OPERATIONS / ACCESS" title="Subscriptions" description="Local entitlement lifecycle and provisioning state." /><DataList data={data} render={(item) => <><div><p>{item.tariff_code || "Trial"}</p><h2>{item.username ? `@${item.username}` : `Telegram ${item.telegram_user_id}`}</h2><small>Expires {date(item.expires_at)}</small></div><b className={`badge ${item.status === "active" ? "ok" : "warn"}`}>{label(item.status)}</b></>} /></>; }
function Catalog({ title, items }) { return <><Heading kicker="OPERATIONS / CATALOG" title={title} description="Manage configuration through audited forms in the next increment." /><section className="data-list">{items.length ? items.map((item) => <article className="data-card" key={item.id || item.code || item.telegram_chat_id}><div><p>{item.code || item.title}</p><h2>{item.name?.ru || item.code || item.title}</h2><small>{item.currency_code || item.telegram_chat_id || item.kind || "Configured item"}</small></div><b className={`badge ${item.is_active !== false ? "ok" : "warn"}`}>{item.is_active === false ? "off" : "active"}</b></article>) : <p className="empty">Nothing to show.</p>}</section></>; }
function DataList({ data, render }) { return <><section className="data-list">{data.items.length ? data.items.map((item) => <article className="data-card" key={item.id}>{render(item)}</article>) : <p className="empty">Nothing to show.</p>}</section><Pager data={data} /></>; }
function Pager({ data }) { return <div className="pager"><span>{data.total ? `${data.offset + 1}-${Math.min(data.offset + data.limit, data.total)} / ${data.total}` : "0 records"}</span></div>; }
function Integrations({ items }) { return <><Heading kicker="ACCESS / INTEGRATIONS" title="Payment providers" description="Secrets are masked and configured in the administrator control plane." /><section className="provider-grid">{items.map((item) => <article className="provider-card" key={item.provider_code}><div><p>{providerNames[item.provider_code]}</p><h2>{item.is_enabled ? "Enabled" : "Disabled"}</h2><small>{item.is_configured ? "Credentials configured" : "Credentials missing"}</small></div><b className={`badge ${item.is_enabled ? "ok" : "warn"}`}>{item.is_enabled ? "live" : "off"}</b></article>)}</section></>; }
function Policies({ settings, referrals, onSave }) { return <><Heading kicker="SETTINGS / COMMERCIAL" title="Policies" description="New entitlements use these server-audited defaults." /><section className="two-panel"><SettingsForm title="Trial access" value={settings} fields={["duration_seconds", "traffic_bytes"]} onSave={(value) => onSave("/admin/trial-settings", numbers(value), "Trial policy saved.")} /><SettingsForm title="Referral reward" value={referrals} fields={["percent"]} onSave={(value) => onSave("/admin/referral-settings", numbers(value), "Referral policy saved.")} /></section></>; }
function Runtime({ data, onSave }) { return <><Heading kicker="SETTINGS / RUNTIME" title="Public URLs and transport" description="Host nginx, DNS and TLS remain manual. Port changes require a Compose restart and matching host nginx update." /><section className="two-panel"><SettingsForm title="Public URLs" value={{ ...data.publicSettings, cors_origins: data.publicSettings.cors_origins.join(", ") }} fields={["mini_app_url", "admin_url", "subscription_public_url", "telegram_webhook_url", "cors_origins", "support_url"]} onSave={(value) => onSave("/admin/public-settings", { ...value, cors_origins: value.cors_origins.split(",").map((item) => item.trim()).filter(Boolean), support_url: value.support_url || null }, "Public URL settings saved.")} /><SettingsForm title="Loopback ports" value={data.runtimeSettings} fields={["api_host_port", "mini_app_host_port", "admin_host_port", "telegram_webhook_host_port"]} onSave={(value) => onSave("/admin/runtime-settings", numbers(value), "Port plan saved. Restart Compose and update host nginx manually.")} /></section><section className="two-panel"><SettingsForm title="Telegram transport" value={data.telegramTransport} fields={["mode"]} options={{ mode: ["polling", "webhook"] }} onSave={(value) => onSave("/admin/telegram-transport", value, "Telegram transport saved. Restart the bot to apply it.")} /><article className="panel"><p>MANUAL HOST BOUNDARY</p><h2>Reverse proxy stays outside</h2><p className="helper">The application records URLs and recommended loopback ports. It never writes nginx configuration, requests certificates, or manages DNS.</p></article></section></>; }
function SettingsForm({ title, value, fields, options, onSave }) { const [form, setForm] = useState(value); useEffect(() => setForm(value), [value]); return <form className="form-card" onSubmit={(event) => { event.preventDefault(); onSave(form); }}><div className="form-head"><div><p>CONFIGURATION</p><h2>{title}</h2></div></div>{fields.map((field) => <label key={field}>{field.replaceAll("_", " ")}{options?.[field] ? <select value={form[field] || ""} onChange={(event) => setForm({ ...form, [field]: event.target.value })}>{options[field].map((option) => <option key={option}>{option}</option>)}</select> : <input value={form[field] ?? ""} onChange={(event) => setForm({ ...form, [field]: event.target.value })} />}</label>)}<div className="form-actions"><button type="submit">Save</button></div></form>; }
function Audit({ data }) { return <><Heading kicker="SETTINGS / AUDIT" title="Audit log" description="Recent privileged changes recorded by the control plane." /><DataList data={data} render={(item) => <><div><p>{item.target_type}</p><h2>{item.action}</h2><small>{date(item.created_at)}</small></div><b className="badge">{item.actor_user_id ? "admin" : "system"}</b></>} /></>; }
function System() { return <><Heading kicker="SETTINGS / SYSTEM" title="System status" description="Operational boundaries for this release." /><section className="system-grid"><article className="panel"><p>AVAILABLE NOW</p><h2>Commercial control plane</h2><ul><li>Invoices and provider webhooks</li><li>Outbox delivery and provisioning leases</li><li>Client-adaptive subscription gateway</li></ul></article><article className="panel"><p>MANUAL OPERATIONS</p><h2>Host administrator</h2><ul><li>Nginx, domains and TLS</li><li>Compose restart after port changes</li><li>Backup retention and restore checks</li></ul></article></section></>; }
const numbers = (value) => Object.fromEntries(Object.entries(value).map(([key, item]) => [key, /^\d+$/.test(String(item)) ? Number(item) : item]));
function previewData() { const empty = { items: [], total: 0, limit: 25, offset: 0 }; return { dashboard: { registered_users: 1284, active_subscriptions: 942, paid_invoices: 1670, paid_revenue_rub_minor: 28743000, pending_invoices: 4, provisioning_pending_subscriptions: 2 }, settings: { duration_seconds: 259200, traffic_bytes: 10000000000 }, referralSettings: { percent: 10 }, publicSettings: { mini_app_url: "https://app.example.com", admin_url: "https://admin.example.com", subscription_public_url: "https://app.example.com", telegram_webhook_url: "https://app.example.com/telegram/webhook", cors_origins: ["https://app.example.com", "https://admin.example.com"], support_url: "https://t.me/support" }, runtimeSettings: { api_host_port: 18080, mini_app_host_port: 18081, admin_host_port: 18082, telegram_webhook_host_port: 18083 }, telegramTransport: { mode: "webhook" }, providers: [{ provider_code: "crypto_pay", is_enabled: true, is_configured: true }, { provider_code: "anore", is_enabled: true, is_configured: true }, { provider_code: "telegram_stars", is_enabled: false, is_configured: true }], tariffs: [{ id: "t1", code: "month", name: { ru: "Месяц" }, currency_code: "RUB", is_active: true }], channels: [], promos: [], users: { ...empty, items: [{ id: "u1", telegram_user_id: 1001, first_name: "Anna", username: "aurora", language_code: "ru", balance_minor: 14900, currency_code: "RUB", created_at: "2026-07-24T09:00:00Z" }], total: 1 }, subscriptions: empty, invoices: empty, audit: empty }; }

createRoot(document.getElementById("app")).render(<StrictMode><App /></StrictMode>);
