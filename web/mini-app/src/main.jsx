import { StrictMode, useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";

const apiBase = window.VPN_API_BASE_URL || "/api/v1";
const telegram = window.Telegram?.WebApp;
const preview = new URLSearchParams(location.search).get("preview") === "1";
const text = {
  ru: { title: "Ваш VPN", plans: "Тарифы", wallet: "Кошелёк", access: "Доступ", renew: "Продлить", trial: "Попробовать 72 часа", choose: "Выбрать тариф", checkout: "Оформление", back: "Назад", copy: "Скопировать ссылку", open: "Открыть доступ", pay: "Оплатить с кошелька", promo: "Промокод", apply: "Применить", loading: "Готовим ваш защищённый маршрут", retry: "Повторить", unavailable: "Откройте приложение из Telegram", active: "Активен до", pending: "Подключаем доступ", inactive: "Доступ ещё не подключён", payment: "Способ оплаты", instructions: "Как подключиться", paymentCreated: "Счёт создан. Завершите оплату в открывшемся окне.", processing: "Операция обрабатывается. Статус обновится автоматически." },
  en: { title: "Your VPN", plans: "Plans", wallet: "Wallet", access: "Access", renew: "Renew", trial: "Try for 72 hours", choose: "Choose plan", checkout: "Checkout", back: "Back", copy: "Copy link", open: "Open access", pay: "Pay with wallet", promo: "Promo code", apply: "Apply", loading: "Preparing your secure route", retry: "Retry", unavailable: "Open this app from Telegram", active: "Active until", pending: "Preparing your access", inactive: "Your access is not connected", payment: "Payment method", instructions: "Connection guide", paymentCreated: "Invoice created. Complete payment in the opened window.", processing: "Your request is processing. Status will update automatically." },
};

function money(value, currency, language) {
  return new Intl.NumberFormat(language === "ru" ? "ru-RU" : "en-US", { style: "currency", currency, maximumFractionDigits: currency === "XTR" ? 0 : 2 }).format(value / (currency === "XTR" ? 1 : 100));
}

function formatDate(value, language) {
  return value ? new Intl.DateTimeFormat(language === "ru" ? "ru-RU" : "en-US", { day: "numeric", month: "short", year: "numeric" }).format(new Date(value)) : "-";
}

function App() {
  const [token, setToken] = useState(null);
  const [data, setData] = useState(null);
  const [screen, setScreen] = useState("home");
  const [selected, setSelected] = useState(null);
  const [accessUrl, setAccessUrl] = useState(null);
  const [promo, setPromo] = useState("");
  const [discount, setDiscount] = useState(null);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const language = data?.me?.language_code || "ru";
  const t = (key) => text[language][key];

  const request = async (path, options = {}) => {
    const response = await fetch(`${apiBase}${path}`, { ...options, headers: { "Content-Type": "application/json", ...(token ? { Authorization: `Bearer ${token}` } : {}), ...options.headers } });
    if (!response.ok) throw new Error((await response.json().catch(() => ({}))).message || "Request failed");
    return response.status === 204 ? null : response.json();
  };

  const refresh = async (sessionToken = token) => {
    if (preview) return;
    const auth = { Authorization: `Bearer ${sessionToken}` };
    const get = async (path) => {
      const response = await fetch(`${apiBase}${path}`, { headers: auth });
      if (!response.ok) throw new Error((await response.json().catch(() => ({}))).message || "Request failed");
      return response.json();
    };
    const [me, subscription, tariffs, providers] = await Promise.all([get("/me"), get("/subscriptions/current"), get("/tariffs"), get("/payment-providers")]);
    setData({ me, subscription, tariffs, providers });
  };

  const bootstrap = async () => {
    setLoading(true); setError("");
    try {
      if (preview) { setData(previewData()); return; }
      telegram?.ready(); telegram?.expand();
      const saved = JSON.parse(sessionStorage.getItem("vpn-mini-app-session") || "null");
      let session = saved?.token && saved.expiresAt > Date.now() ? saved.token : null;
      if (!session) {
        if (!telegram?.initData) throw new Error(t("unavailable"));
        const response = await fetch(`${apiBase}/auth/telegram`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ init_data: telegram.initData }) });
        if (!response.ok) throw new Error((await response.json().catch(() => ({}))).message || "Authentication failed");
        const auth = await response.json();
        session = auth.access_token;
        sessionStorage.setItem("vpn-mini-app-session", JSON.stringify({ token: session, expiresAt: new Date(auth.expires_at).getTime() }));
      }
      setToken(session); await refresh(session);
    } catch (cause) { sessionStorage.removeItem("vpn-mini-app-session"); setError(cause.message); }
    finally { setLoading(false); }
  };

  useEffect(() => { bootstrap(); }, []);
  const selectedTariff = useMemo(() => data?.tariffs.find((item) => item.id === selected) || null, [data, selected]);

  const mutate = async (action, success) => {
    try { await action(); await success(); } catch (cause) { setNotice(cause.message); }
  };
  const openAccess = () => { if (preview) { setAccessUrl("http://31.77.203.80:18081/sub/demo?token=preview-only"); setScreen("access"); return; } return mutate(async () => {
    const response = await request(`/subscriptions/${data.subscription.id}/access`); setAccessUrl(response.access_url); setScreen("access");
  }, async () => {}); };
  const changeLanguage = () => { if (preview) { setData((current) => ({ ...current, me: { ...current.me, language_code: language === "ru" ? "en" : "ru" } })); return; } return mutate(async () => {
    const me = await request("/me", { method: "PUT", body: JSON.stringify({ language_code: language === "ru" ? "en" : "ru" }) });
    setData((current) => ({ ...current, me }));
  }, async () => {}); };
  const purchase = () => { if (preview) { setNotice(t("processing")); setScreen("home"); return; } return mutate(() => request("/purchases", { method: "POST", headers: { "Idempotency-Key": crypto.randomUUID() }, body: JSON.stringify({ tariff_id: selected, promo_code: promo || null }) }), async () => { setNotice(t("processing")); setPromo(""); setDiscount(null); setScreen("home"); await refresh(); }); };
  const trial = () => { if (preview) { setNotice(t("processing")); return; } return mutate(() => request("/trials", { method: "POST" }), async () => { setNotice(t("processing")); await refresh(); }); };
  const redeem = () => mutate(async () => {
    const preview = await request(`/promos/${encodeURIComponent(promo.trim())}/preview`);
    if (preview.kind === "discount") { setPromo(preview.code); setDiscount(preview.discount_percent); setNotice(`-${preview.discount_percent}%`); return; }
    const result = await request("/promos/redeem", { method: "POST", body: JSON.stringify({ code: promo.trim() }) });
    setPromo(""); setNotice(`${result.code}: +${money(result.credited_amount_minor, result.currency_code, language)}`); await refresh();
  }, async () => {});
  const invoice = (provider) => { if (preview) { setNotice(t("paymentCreated")); setScreen("home"); return; } return mutate(async () => {
    const result = await request("/invoices", { method: "POST", body: JSON.stringify({ provider, currency_code: selectedTariff.currency_code, tariff_id: selected }) });
    if (provider === "telegram_stars" && telegram?.openInvoice) telegram.openInvoice(result.payment_url, refresh);
    else if (telegram?.openLink) telegram.openLink(result.payment_url); else window.open(result.payment_url, "_blank", "noopener,noreferrer");
  }, async () => { setNotice(t("paymentCreated")); setScreen("home"); }); };

  if (loading) return <State title={t("loading")} />;
  if (error) return <State title={error} action={bootstrap} actionLabel={t("retry")} error />;
  const { me, subscription, tariffs, providers } = data;
  const tariffTitle = (item) => item.name[language] || item.name.ru || item.name.en || item.code;
  const amount = selectedTariff && discount ? Math.floor(selectedTariff.amount_minor * (100 - discount) / 100) : selectedTariff?.amount_minor;
  return <main className="mini-shell">
    <header className="mini-topbar"><button className="wordmark" onClick={() => setScreen("home")}><i>V</i>VECTOR</button><button className="lang" onClick={changeLanguage}>{language === "ru" ? "EN" : "RU"}</button></header>
    {notice && <button className="mini-notice" onClick={() => setNotice("")}>{notice}<b>×</b></button>}
    {screen === "home" && <Home subscription={subscription} me={me} language={language} t={t} onAccess={openAccess} onCatalog={() => setScreen("catalog")} onTrial={trial} />}
    {screen === "catalog" && <Catalog tariffs={tariffs} selected={selected} language={language} t={t} title={tariffTitle} onBack={() => setScreen("home")} onSelect={setSelected} onContinue={() => setScreen("checkout")} />}
    {screen === "checkout" && <Checkout tariff={selectedTariff} providers={providers} me={me} promo={promo} discount={discount} amount={amount} language={language} t={t} title={tariffTitle} onBack={() => setScreen("catalog")} onPromo={setPromo} onRedeem={redeem} onWallet={purchase} onInvoice={invoice} />}
    {screen === "access" && <Access url={accessUrl} language={language} t={t} onBack={() => setScreen("home")} onCopy={() => navigator.clipboard.writeText(accessUrl).then(() => setNotice(language === "ru" ? "Ссылка скопирована" : "Link copied"))} />}
  </main>;
}

function State({ title, action, actionLabel, error }) { return <main className="mini-shell state-screen"><span className={error ? "state-mark error" : "state-mark"}>{error ? "!" : ""}</span><h1>{title}</h1>{action && <button className="primary" onClick={action}>{actionLabel}</button>}</main>; }
function Home({ subscription, me, language, t, onAccess, onCatalog, onTrial }) {
  const active = subscription?.status === "active";
  const status = active ? t("active") : subscription?.status === "provisioning_pending" ? t("pending") : t("inactive");
  return <><section className="route-hero"><div className="route-grid" /><p>SECURE ROUTE / 01</p><h1>{t("title")}</h1><div className="route-status"><i className={active ? "on" : ""} />{status}</div>{active && <strong>{formatDate(subscription.expires_at, language)}</strong>}<div className="route-actions"><button className="primary" disabled={!subscription?.access_available} onClick={onAccess}>{t("open")}</button><button className="secondary" onClick={onCatalog}>{t("renew")}</button></div></section>{!subscription && <button className="trial-cta" onClick={onTrial}>{t("trial")}<span>10 GB →</span></button>}<section className="balance-card"><div><span>{t("wallet")}</span><b>{money(me.balance_minor, me.currency_code, language)}</b></div><button onClick={onCatalog}>{language === "ru" ? "Пополнить" : "Top up"}</button></section><section className="mini-section"><p>PLAN / 02</p><button onClick={onCatalog}>{t("plans")}</button></section></>;
}
function Catalog({ tariffs, selected, language, t, title, onBack, onSelect, onContinue }) { return <><ScreenHead title={t("plans")} label="PLAN / 02" onBack={onBack} /><section className="plan-stack">{tariffs.map((tariff) => <button className={`plan-card ${selected === tariff.id ? "selected" : ""}`} key={tariff.id} onClick={() => onSelect(tariff.id)}><span>{title(tariff)}</span><b>{money(tariff.amount_minor, tariff.currency_code, language)}</b><small>{tariff.duration_seconds ? `${Math.round(tariff.duration_seconds / 86400)} ${language === "ru" ? "дней" : "days"}` : ""}{tariff.traffic_bytes ? ` · ${Math.round(tariff.traffic_bytes / 1e9)} GB` : ""}</small></button>)}</section><button className="primary sticky" disabled={!selected} onClick={onContinue}>{t("choose")}</button></>; }
function Checkout({ tariff, providers, me, promo, discount, amount, language, t, title, onBack, onPromo, onRedeem, onWallet, onInvoice }) { const available = providers.filter((item) => item.supported_currency_codes.includes(tariff.currency_code)); return <><ScreenHead title={t("checkout")} label="CHECKOUT / 03" onBack={onBack} /><section className="checkout-card"><span>{title(tariff)}</span><b>{money(tariff.amount_minor, tariff.currency_code, language)}</b><div><small>{language === "ru" ? "Итого" : "Total"}{discount && ` · -${discount}%`}</small><strong>{money(amount, tariff.currency_code, language)}</strong></div></section><form className="promo-row" onSubmit={(event) => { event.preventDefault(); if (promo.trim()) onRedeem(); }}><input value={promo} maxLength="64" placeholder={t("promo")} onChange={(event) => onPromo(event.target.value)} /><button>{t("apply")}</button></form>{tariff.currency_code === me.currency_code && <button className="primary full" onClick={onWallet}>{t("pay")}</button>}<section className="provider-list"><p>{t("payment")}</p>{discount ? <small>{language === "ru" ? "Скидка применяется при оплате с кошелька." : "Discount applies to wallet purchases."}</small> : available.map((provider) => <button key={provider.code} onClick={() => onInvoice(provider.code)}><span>{provider.code.replaceAll("_", " ")}</span>→</button>)}</section></>; }
function Access({ url, t, onBack, onCopy }) { return <><ScreenHead title={t("access")} label="ROUTE / 04" onBack={onBack} /><section className="access-card"><span>CONNECTION LINK</span><code>{url}</code><button className="primary full" onClick={onCopy}>{t("copy")}</button></section><section className="guide-card"><p>01 / 02 / 03</p><h2>{t("instructions")}</h2><ol><li>Install Happ, v2rayNG, Karing, or Clash.</li><li>Copy and import this protected link.</li><li>Connect. Your client receives a compatible format.</li></ol></section></>; }
function ScreenHead({ label, title, onBack }) { return <header className="screen-head"><button onClick={onBack}>←</button><div><p>{label}</p><h1>{title}</h1></div></header>; }
function previewData() { return { me: { language_code: "ru", balance_minor: 14900, currency_code: "RUB" }, subscription: { id: "preview-subscription", status: "active", expires_at: "2026-08-24T12:00:00Z", traffic_bytes: 100000000000, access_available: true }, tariffs: [{ id: "month", code: "month", name: { ru: "Месяц", en: "One month" }, amount_minor: 29900, currency_code: "RUB", duration_seconds: 2592000, traffic_bytes: 100000000000 }, { id: "quarter", code: "quarter", name: { ru: "Три месяца", en: "Three months" }, amount_minor: 79900, currency_code: "RUB", duration_seconds: 7776000, traffic_bytes: 300000000000 }], providers: [{ code: "crypto_pay", supported_currency_codes: ["RUB"] }, { code: "anore", supported_currency_codes: ["RUB"] }] }; }

createRoot(document.getElementById("app")).render(<StrictMode><App /></StrictMode>);
