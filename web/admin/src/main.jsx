import { StrictMode, useEffect, useState, useCallback, useRef, useMemo } from "react";
import { createRoot } from "react-dom/client";
import "../styles.css";

const apiBase = window.VPN_API_BASE_URL || "/api/v1";
const providerNames = { crypto_pay: "Crypto Pay", anore: "Anore", telegram_stars: "Telegram Stars" };
const money = (value, currency = "RUB") => new Intl.NumberFormat("ru-RU", { style: "currency", currency, maximumFractionDigits: currency === "XTR" ? 0 : 2 }).format((value || 0) / (currency === "XTR" ? 1 : 100));
const date = (value) => value ? new Intl.DateTimeFormat("ru-RU", { day: "2-digit", month: "short", year: "numeric" }).format(new Date(value)) : "-";
const timeAgo = (value) => {
  if (!value) return "";
  const diff = Date.now() - new Date(value).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "только что";
  if (mins < 60) return `${mins} мин назад`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs} ч назад`;
  const days = Math.floor(hrs / 24);
  return `${days} дн назад`;
};
const label = (value) => ({ paid: "оплачен", pending: "ожидает", active: "активна", provisioning_pending: "в обработке", expired: "истекла", suspended: "приостановлена", failed: "ошибка", wallet_top_up: "пополнение", direct_purchase: "покупка", balance: "баланс", discount: "скидка" }[value] || String(value || "").replaceAll("_", " "));
const initialTariff = { code: "", name: "", description: "", duration_days: "30", traffic_gb: "", position: "0", amount_rub: "", currency_code: "RUB", is_active: true };
const initialPromo = { code: "", kind: "discount", amount_rub: "", discount_percent: "", maximum_redemptions: "", is_active: true };
const initialChannel = { telegram_chat_id: "", title: "", public_url: "", is_active: true };

const presets = ["matrix", "cyan", "violet", "amber", "red", "frost"];
const densities = ["compact", "comfortable", "spacious"];

function State({ title, detail, action }) {
  return (
    <div className="state">
      <h1>{title}</h1>
      {detail && <p>{detail}</p>}
      {action && <button onClick={action}>Повторить</button>}
    </div>
  );
}

function App() {
  const [token, setToken] = useState(null);
  const [data, setData] = useState(null);
  const [tab, setTab] = useState("overview");
  const [setupRequired, setSetupRequired] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [cmdOpen, setCmdOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [preset, setPreset] = useState(() => localStorage.getItem("vpn-preset") || "frost");
  const [density, setDensity] = useState(() => localStorage.getItem("vpn-density") || "comfortable");
  const [mobileNav, setMobileNav] = useState(false);

  useEffect(() => {
    document.documentElement.setAttribute("data-preset", preset);
    localStorage.setItem("vpn-preset", preset);
  }, [preset]);
  useEffect(() => {
    document.documentElement.setAttribute("data-density", density);
    localStorage.setItem("vpn-density", density);
  }, [density]);

  const request = async (path, options = {}, session = token) => {
    const response = await fetch(`${apiBase}${path}`, { ...options, headers: { "Content-Type": "application/json", ...(session ? { Authorization: `Bearer ${session}` } : {}), ...options.headers } });
    if (!response.ok) throw new Error((await response.json().catch(() => ({}))).message || "Не удалось выполнить запрос");
    return response.status === 204 ? null : response.json();
  };

  const load = async (session) => {
    const get = (path) => request(path, {}, session);
    const [dashboard, analytics, settings, referralSettings, providers, publicSettings, runtimeSettings, telegramTransport, tariffs, channels, promos, users, subscriptions, invoices, audit] = await Promise.all([
      get("/admin/dashboard"), get("/admin/analytics"), get("/admin/trial-settings"), get("/admin/referral-settings"), get("/admin/payment-providers"), get("/admin/public-settings"), get("/admin/runtime-settings"), get("/admin/telegram-transport"), get("/admin/tariffs"), get("/admin/required-channels"), get("/admin/promos"), get("/admin/users?limit=50&offset=0"), get("/admin/subscriptions?limit=50&offset=0"), get("/admin/invoices?limit=50&offset=0"), get("/admin/audit?limit=50&offset=0")
    ]);
    setData({ dashboard, analytics, settings, referralSettings, providers, publicSettings, runtimeSettings, telegramTransport, tariffs, channels, promos, users, subscriptions, invoices, audit });
  };

  const bootstrap = async () => {
    setLoading(true); setError("");
    try {
      const saved = JSON.parse(sessionStorage.getItem("vpn-admin-session") || "null");
      if (saved?.token && saved.expiresAt > Date.now()) {
        setToken(saved.token); await load(saved.token);
      } else {
        setSetupRequired((await request("/admin/setup-status")).setup_required);
      }
    } catch (cause) { setError(cause.message); } finally { setLoading(false); }
  };

  useEffect(() => { bootstrap(); }, []);

  useEffect(() => {
    const handler = (e) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "k") { e.preventDefault(); setCmdOpen((v) => !v); }
      if (e.key === "Escape") setCmdOpen(false);
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  const authenticate = async ({ login, password }) => {
    setLoading(true); setError("");
    try {
      const auth = await request(setupRequired ? "/admin/setup" : "/admin/login", { method: "POST", body: JSON.stringify({ login, password }) }, null);
      sessionStorage.setItem("vpn-admin-session", JSON.stringify({ token: auth.access_token, expiresAt: new Date(auth.expires_at).getTime() }));
      setToken(auth.access_token); await load(auth.access_token);
    } catch (cause) { setError(cause.message); } finally { setLoading(false); }
  };

  const mutate = async (path, method, body, message) => {
    try { await request(path, { method, body: JSON.stringify(body) }); setNotice(message); await load(token); } catch (cause) { setNotice(cause.message); }
  };

  if (loading) return <State title="Загружаем панель управления" />;
  if (!token) return <AdminLogin setupRequired={setupRequired} error={error} onSubmit={authenticate} />;
  if (error) return <State title="Нет доступа к панели" detail={error} action={bootstrap} />;

  const navItems = [
    ["ГЛАВНОЕ", [["overview", "Обзор", "📊"], ["users", "Клиенты", "👤"]]],
    ["ОПЕРАЦИИ", [["payments", "Платежи", "💰"], ["subscriptions", "Подписки", "🔑"], ["tariffs", "Тарифы", "📋"], ["promos", "Промокоды", "🏷️"]]],
    ["ДОСТУП", [["channels", "Каналы", "📢"], ["integrations", "Платежные системы", "🔌"]]],
    ["НАСТРОЙКИ", [["policies", "Правила", "⚙️"], ["runtime", "Среда", "🖥️"], ["audit", "Журнал", "📝"]]]
  ];

  const flatNav = navItems.flatMap(([, items]) => items);

  const pages = {
    overview: <Overview dashboard={data.dashboard} analytics={data.analytics} audit={data.audit} />,
    users: <Users data={data.users} onCreate={(body) => mutate("/admin/users", "POST", body, "Клиент создан.")} />,
    payments: <Invoices data={data.invoices} />,
    subscriptions: <Subscriptions data={data.subscriptions} />,
    tariffs: <Tariffs items={data.tariffs} onSave={(body, id) => mutate(`/admin/tariffs${id ? `/${id}` : ""}`, id ? "PUT" : "POST", body, id ? "Тариф обновлен." : "Тариф создан.")} />,
    promos: <Promos items={data.promos} onSave={(body, id) => mutate(`/admin/promos${id ? `/${id}` : ""}`, id ? "PUT" : "POST", body, id ? "Промокод обновлен." : "Промокод создан.")} />,
    channels: <Channels items={data.channels} onSave={(body, id) => mutate(`/admin/required-channels${id ? `/${id}` : ""}`, id ? "PUT" : "POST", body, id ? "Канал обновлен." : "Канал добавлен.")} />,
    integrations: <Integrations items={data.providers} onToggle={(item) => mutate(`/admin/payment-providers/${item.provider_code}`, "PUT", { is_enabled: !item.is_enabled }, item.is_enabled ? "Провайдер выключен." : "Провайдер включен.")} />,
    policies: <Policies settings={data.settings} referrals={data.referralSettings} onSave={(path, body, message) => mutate(path, "PUT", body, message)} />,
    runtime: <Runtime data={data} onSave={(path, body, message) => mutate(path, "PUT", body, message)} />,
    audit: <Audit data={data.audit} />
  };

  const navigate = (id) => { setTab(id); setCmdOpen(false); setMobileNav(false); };

  return (
    <div className="shell">
      <aside className={`sidebar ${mobileNav ? "open" : ""}`}>
        <div className="sidebar-brand"><b /><div>VPN // OPS <span>УПРАВЛЕНИЕ</span></div></div>
        <nav className="sidebar-nav">
          {navItems.map(([group, items]) => (
            <div className="nav-group" key={group}>
              <small>{group}</small>
              {items.map(([id, title, icon]) => (
                <button className={`nav-item ${tab === id ? "active" : ""}`} onClick={() => navigate(id)} key={id}>
                  <span>{icon}</span> {title}
                </button>
              ))}
            </div>
          ))}
        </nav>
        <div className="sidebar-foot">ADMIN / V2<br />АУДИТИРУЕМЫЕ ОПЕРАЦИИ</div>
      </aside>
      <div className="main">
        <header className="topbar">
          <div className="topbar-left">
            <button className="btn-ghost" onClick={() => setMobileNav(!mobileNav)} style={{ display: "none" }} id="mobile-toggle">☰</button>
            <button className="cmd-trigger" onClick={() => setCmdOpen(true)}>
              🔍 <span>Поиск страниц и действий...</span>
              <kbd>⌘K</kbd>
            </button>
          </div>
          <div className="topbar-right">
            <span className="live-dot">СЕССИЯ АКТИВНА</span>
            <div style={{ position: "relative" }}>
              <button className="btn-ghost" onClick={() => setSettingsOpen(!settingsOpen)}>⚙ Настройки</button>
              {settingsOpen && <SettingsPopover preset={preset} setPreset={setPreset} density={density} setDensity={setDensity} onClose={() => setSettingsOpen(false)} />}
            </div>
            <button className="btn-ghost" onClick={() => { sessionStorage.removeItem("vpn-admin-session"); setToken(null); setData(null); bootstrap(); }}>Выйти</button>
          </div>
        </header>
        {cmdOpen && <CommandPalette navItems={flatNav} onNavigate={navigate} onClose={() => setCmdOpen(false)} />}
        <main className="content">
          {notice && <button className="notice" onClick={() => setNotice("")}>{notice}<span>×</span></button>}
          {pages[tab]}
        </main>
      </div>
    </div>
  );
}

function SettingsPopover({ preset, setPreset, density, setDensity, onClose }) {
  const ref = useRef(null);
  useEffect(() => {
    const handler = (e) => { if (ref.current && !ref.current.contains(e.target)) onClose(); };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [onClose]);
  const presetColors = { matrix: "#39d353", cyan: "#22d3ee", violet: "#a78bfa", amber: "#fbbf24", red: "#f87171", frost: "#94a3b8" };
  return (
    <div className="settings-popover" ref={ref}>
      <h3>Цветовая схема</h3>
      <div className="preset-dots">
        {presets.map((p) => (
          <button key={p} className={`preset-dot ${preset === p ? "active" : ""}`} style={{ background: presetColors[p] }} onClick={() => setPreset(p)} title={p} />
        ))}
      </div>
      <h3 style={{ marginTop: 16 }}>Плотность</h3>
      <label>
        Режим
        <select value={density} onChange={(e) => setDensity(e.target.value)}>
          {densities.map((d) => <option key={d} value={d}>{d === "compact" ? "Компактный" : d === "comfortable" ? "Комфортный" : "Просторный"}</option>)}
        </select>
      </label>
    </div>
  );
}

function CommandPalette({ navItems, onNavigate, onClose }) {
  const [query, setQuery] = useState("");
  const [focusIdx, setFocusIdx] = useState(0);
  const inputRef = useRef(null);

  useEffect(() => { inputRef.current?.focus(); }, []);

  const filtered = useMemo(() => {
    if (!query) return navItems;
    const q = query.toLowerCase();
    return navItems.filter(([, title]) => title.toLowerCase().includes(q));
  }, [query, navItems]);

  useEffect(() => { setFocusIdx(0); }, [query]);

  const handleKey = (e) => {
    if (e.key === "ArrowDown") { e.preventDefault(); setFocusIdx((i) => Math.min(i + 1, filtered.length - 1)); }
    if (e.key === "ArrowUp") { e.preventDefault(); setFocusIdx((i) => Math.max(i - 1, 0)); }
    if (e.key === "Enter" && filtered[focusIdx]) { onNavigate(filtered[focusIdx][0]); }
  };

  return (
    <div className="cmd-overlay" onClick={onClose}>
      <div className="cmd-modal" onClick={(e) => e.stopPropagation()}>
        <input ref={inputRef} className="cmd-input" placeholder="Введите команду или страницу..." value={query} onChange={(e) => setQuery(e.target.value)} onKeyDown={handleKey} />
        <div className="cmd-results">
          {filtered.length ? filtered.map(([id, title, icon], idx) => (
            <button key={id} className={`cmd-item ${idx === focusIdx ? "focused" : ""}`} onClick={() => onNavigate(id)}>
              <span className="cmd-icon">{icon}</span>
              <span className="cmd-label">{title}</span>
            </button>
          )) : <div className="cmd-empty">Ничего не найдено</div>}
        </div>
      </div>
    </div>
  );
}

function AdminLogin({ setupRequired, error, onSubmit }) {
  const [login, setLogin] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [localError, setLocalError] = useState("");
  const submit = (event) => {
    event.preventDefault();
    if (setupRequired && password !== confirm) return setLocalError("Пароли не совпадают.");
    onSubmit({ login, password });
  };
  return (
    <main className="login-shell">
      <section className="login-card">
        <p className="eyebrow">VPN // CONTROL PLANE</p>
        <h1>{setupRequired ? "Создание главного администратора" : "Вход администратора"}</h1>
        <p className="helper">{setupRequired ? "Эта учетная запись управляет доступом в панель." : "Используйте логин и пароль администратора."}</p>
        <form onSubmit={submit}>
          <label>Логин<input value={login} autoComplete="username" minLength={3} onChange={(e) => setLogin(e.target.value)} required /></label>
          <label>Пароль<input value={password} type="password" autoComplete="current-password" minLength={12} onChange={(e) => setPassword(e.target.value)} required /></label>
          {setupRequired && <label>Повторите пароль<input value={confirm} type="password" minLength={12} onChange={(e) => setConfirm(e.target.value)} required /></label>}
          <button type="submit">{setupRequired ? "Создать администратора" : "Войти"}</button>
        </form>
        {(localError || error) && <p className="login-error">{localError || error}</p>}
      </section>
    </main>
  );
}

function Overview({ dashboard, analytics, audit }) {
  const recentAudits = audit?.items?.slice(0, 5) || [];
  return (
    <>
      <section className="section-head">
        <div><p>ГЛАВНОЕ / ОБЗОР</p><h1>Операционный центр</h1><span>Ключевые показатели, денежный поток и регистрация клиентов.</span></div>
      </section>
      <section className="metric-grid">
        <MetricCard icon="👥" label="Клиенты" value={dashboard.registered_users} sub="Всего зарегистрировано" />
        <MetricCard icon="🔑" label="Активные подписки" value={dashboard.active_subscriptions} sub="Сейчас активны" />
        <MetricCard icon="💰" label="Выручка" value={money(dashboard.paid_revenue_rub_minor)} sub="Оплаченные счета" />
        <MetricCard icon="⏳" label="Ожидают" value={dashboard.provisioning_pending_subscriptions + dashboard.pending_invoices} sub="Требуют внимания" trend={dashboard.provisioning_pending_subscriptions > 0 ? "down" : "up"} trendLabel={dashboard.provisioning_pending_subscriptions > 0 ? "Есть очереди" : "Система в норме"} />
      </section>
      <section className="chart-grid">
        <Chart title="Регистрации за 14 дней" points={analytics.registrations} />
        <Chart title="Выручка за 14 дней" points={analytics.revenue_rub_minor} format={(v) => money(v)} />
      </section>
      <section className="pipeline-grid">
        <div className="pipeline">
          <div className="panel-title">
            <div><p>ОПЕРАЦИИ</p><h2>Последние действия</h2></div>
          </div>
          <div className="pipeline-items">
            {recentAudits.length ? recentAudits.map((item) => (
              <div className="pipeline-item" key={item.id}>
                <span className={`pi-status ${item.actor_user_id ? "ok" : "warn"}`} />
                <span className="pi-name">{item.action} — {item.target_type}</span>
                <span className="pi-time">{timeAgo(item.created_at)}</span>
                <span className={`pi-badge ${item.actor_user_id ? "ok" : "warn"}`}>{item.actor_user_id ? "админ" : "система"}</span>
              </div>
            )) : <div className="empty">Нет действий</div>}
          </div>
        </div>
      </section>
      <section className="split-grid">
        <article className="panel">
          <div className="panel-title"><div><p>ОЧЕРЕДЬ ОПЕРАТОРА</p><h2>Требует внимания</h2></div></div>
          <div className="signal-row"><span>Ожидают оплаты</span><b>{dashboard.pending_invoices}</b></div>
          <div className="signal-row"><span>Ожидают выдачи доступа</span><b>{dashboard.provisioning_pending_subscriptions}</b></div>
          <div className="signal-row"><span>Оплаченные счета</span><b>{dashboard.paid_invoices}</b></div>
        </article>
        <article className="panel">
          <p>ГРАНИЦА УПРАВЛЕНИЯ</p>
          <h2>Прокси остается на хосте</h2>
          <p className="helper">Домены, nginx и TLS управляются вручную на сервере. В панели редактируются только настройки приложения.</p>
        </article>
      </section>
    </>
  );
}

function MetricCard({ icon, label, value, sub, trend, trendLabel }) {
  return (
    <article className="metric-card">
      <div className="mc-head">
        <span className="mc-label">{label}</span>
        <span className="mc-icon">{icon}</span>
      </div>
      <div className="mc-value">{value}</div>
      <div className="mc-sub">
        {sub}
        {trend && <span className={`mc-trend ${trend}`}>{trend === "up" ? "↑" : "↓"} {trendLabel}</span>}
      </div>
    </article>
  );
}

function Chart({ title, points, format = (v) => v }) {
  const max = Math.max(1, ...points.map((p) => Number(p.value)));
  return (
    <article className="chart">
      <div className="panel-title">
        <div><p>АНАЛИТИКА</p><h2>{title}</h2></div>
        <b>{format(points.reduce((s, p) => s + Number(p.value), 0))}</b>
      </div>
      <div className="bar-chart">
        {points.map((p) => (
          <div key={p.day} title={`${p.day}: ${format(p.value)}`}>
            <i style={{ height: `${Math.max(3, Number(p.value) / max * 100)}%` }} />
            <small>{p.day.slice(5)}</small>
          </div>
        ))}
      </div>
    </article>
  );
}

function Users({ data, onCreate }) {
  const [form, setForm] = useState({ telegram_user_id: "", username: "", first_name: "", language_code: "ru" });
  const submit = (event) => {
    event.preventDefault();
    onCreate({ ...form, telegram_user_id: Number(form.telegram_user_id), username: form.username.replace(/^@/, "") || null });
    setForm({ telegram_user_id: "", username: "", first_name: "", language_code: "ru" });
  };
  return (
    <>
      <section className="section-head"><div><p>ГЛАВНОЕ / КЛИЕНТЫ</p><h1>Клиенты</h1><span>Клиенты обычно появляются после первого входа через Telegram. Здесь можно добавить клиента вручную.</span></div></section>
      <section className="create-card">
        <h2>Добавить клиента</h2>
        <form className="inline-form" onSubmit={submit}>
          <label>Telegram ID<input required inputMode="numeric" placeholder="123456789" value={form.telegram_user_id} onChange={(e) => setForm({ ...form, telegram_user_id: e.target.value })} /></label>
          <label>Username<input placeholder="@username" value={form.username} onChange={(e) => setForm({ ...form, username: e.target.value })} /></label>
          <label>Имя<input required placeholder="Иван" value={form.first_name} onChange={(e) => setForm({ ...form, first_name: e.target.value })} /></label>
          <label>Язык<select value={form.language_code} onChange={(e) => setForm({ ...form, language_code: e.target.value })}><option value="ru">Русский</option><option value="en">English</option></select></label>
          <button>Создать клиента</button>
        </form>
      </section>
      <section className="card-grid">
        {data.items.length ? data.items.map((item) => (
          <article className="user-card" key={item.id}>
            <div><span className="avatar">{(item.first_name || "?")[0]}</span><span><strong>{item.username ? `@${item.username}` : item.first_name}</strong><small>Telegram ID: {item.telegram_user_id}</small></span></div>
            <b className="badge">{item.language_code}</b>
            <div className="card-stats">
              <span><small>Баланс</small><b>{money(item.balance_minor, item.currency_code)}</b></span>
              <span><small>Создан</small><b>{date(item.created_at)}</b></span>
            </div>
          </article>
        )) : <p className="empty">Клиентов пока нет.</p>}
      </section>
    </>
  );
}

function Invoices({ data }) {
  return (
    <>
      <section className="section-head"><div><p>ОПЕРАЦИИ / ПЛАТЕЖИ</p><h1>Платежи</h1><span>Состояния счетов и история платежных провайдеров.</span></div></section>
      <DataList data={data} render={(item) => (
        <><div><p>{providerNames[item.provider] || label(item.provider)}</p><h2>{money(item.amount_minor, item.currency_code)}</h2><small>{label(item.purpose)} · {date(item.created_at)} · {item.username ? `@${item.username}` : `Telegram ${item.telegram_user_id}`}</small></div><b className={`badge ${item.status === "paid" ? "ok" : "warn"}`}>{label(item.status)}</b></>
      )} />
    </>
  );
}

function Subscriptions({ data }) {
  return (
    <>
      <section className="section-head"><div><p>ОПЕРАЦИИ / ДОСТУП</p><h1>Подписки</h1><span>Жизненный цикл доступа и состояние выдачи конфигурации.</span></div></section>
      <DataList data={data} render={(item) => (
        <><div><p>{item.tariff_code || "Пробный доступ"}</p><h2>{item.username ? `@${item.username}` : `Telegram ${item.telegram_user_id}`}</h2><small>Действует до: {date(item.expires_at)}</small></div><b className={`badge ${item.status === "active" ? "ok" : "warn"}`}>{label(item.status)}</b></>
      )} />
    </>
  );
}

function Tariffs({ items, onSave }) {
  const encode = (form) => ({ code: form.code, name: { ru: form.name }, description: { ru: form.description }, duration_seconds: Number(form.duration_days) * 86400 || null, traffic_bytes: form.traffic_gb === "" ? null : Number(form.traffic_gb) * 1024 ** 3, position: Number(form.position || 0), is_active: form.is_active, amount_minor: Math.round(Number(form.amount_rub || 0) * 100), currency_code: form.currency_code });
  const decode = (item) => ({ code: item.code, name: item.name?.ru || "", description: item.description?.ru || "", duration_days: item.duration_seconds ? item.duration_seconds / 86400 : "", traffic_gb: item.traffic_bytes == null ? "" : item.traffic_bytes / 1024 ** 3, position: item.position, amount_rub: item.amount_minor / 100, currency_code: item.currency_code, is_active: item.is_active });
  return <ResourcePage kicker="ОПЕРАЦИИ / КАТАЛОГ" title="Тарифы" description="Создание и редактирование предложений для Mini App." items={items} initial={initialTariff} decode={decode} onSave={(form, id) => onSave(encode(form), id)} fields={[["code", "Код тарифа"], ["name", "Название"], ["description", "Описание"], ["duration_days", "Срок, дней", "number"], ["traffic_gb", "Трафик, ГБ", "number"], ["amount_rub", "Цена, ₽", "number"], ["position", "Позиция", "number"], ["currency_code", "Валюта"]]} render={(item) => (
    <><div><p>{item.code}</p><h2>{item.name?.ru || item.code}</h2><small>{money(item.amount_minor, item.currency_code)} · {item.duration_seconds ? `${item.duration_seconds / 86400} дн.` : "без срока"} · {item.traffic_bytes == null ? "без лимита" : `${Math.round(item.traffic_bytes / 1024 ** 3)} ГБ`}</small></div><b className={`badge ${item.is_active ? "ok" : "warn"}`}>{item.is_active ? "активен" : "выключен"}</b></>
  )} />;
}

function Promos({ items, onSave }) {
  const encode = (form) => ({ code: form.code.toUpperCase(), kind: form.kind, amount_minor: form.kind === "balance" ? Math.round(Number(form.amount_rub || 0) * 100) : null, discount_percent: form.kind === "discount" ? Number(form.discount_percent) : null, maximum_redemptions: form.maximum_redemptions === "" ? null : Number(form.maximum_redemptions), is_active: form.is_active, starts_at: null, ends_at: null });
  const decode = (item) => ({ code: item.code, kind: item.kind, amount_rub: item.amount_minor ? item.amount_minor / 100 : "", discount_percent: item.discount_percent ?? "", maximum_redemptions: item.maximum_redemptions ?? "", is_active: item.is_active });
  return <ResourcePage kicker="ОПЕРАЦИИ / МАРКЕТИНГ" title="Промокоды" description="Фиксированные начисления или скидки на оплату." items={items} initial={initialPromo} decode={decode} onSave={(form, id) => onSave(encode(form), id)} fields={[["code", "Код"], ["kind", "Тип", "select", [["discount", "Скидка"], ["balance", "Баланс"]]], ["discount_percent", "Скидка, %", "number"], ["amount_rub", "Начисление, ₽", "number"], ["maximum_redemptions", "Лимит активаций", "number"]]} render={(item) => (
    <><div><p>{label(item.kind)}</p><h2>{item.code}</h2><small>{item.kind === "balance" ? money(item.amount_minor) : `${item.discount_percent}% скидка`} · использовано {item.redeemed_count}/{item.maximum_redemptions || "∞"}</small></div><b className={`badge ${item.is_active ? "ok" : "warn"}`}>{item.is_active ? "активен" : "выключен"}</b></>
  )} />;
}

function Channels({ items, onSave }) {
  const encode = (form) => ({ ...form, telegram_chat_id: Number(form.telegram_chat_id), public_url: form.public_url || null });
  return <ResourcePage kicker="ДОСТУП / TELEGRAM" title="Обязательные каналы" description="Без подписки на активные каналы клиент не получит доступ к коммерческим действиям." items={items} initial={initialChannel} decode={(item) => ({ telegram_chat_id: item.telegram_chat_id, title: item.title, public_url: item.public_url || "", is_active: item.is_active })} onSave={(form, id) => onSave(encode(form), id)} fields={[["telegram_chat_id", "Telegram chat ID", "number"], ["title", "Название"], ["public_url", "Ссылка t.me"]]} render={(item) => (
    <><div><p>TELEGRAM</p><h2>{item.title}</h2><small>{item.telegram_chat_id}{item.public_url ? ` · ${item.public_url}` : ""}</small></div><b className={`badge ${item.is_active ? "ok" : "warn"}`}>{item.is_active ? "активен" : "выключен"}</b></>
  )} />;
}

function ResourcePage({ kicker, title, description, items, initial, decode, onSave, fields, render }) {
  const [form, setForm] = useState(initial);
  const [editing, setEditing] = useState(null);
  const submit = (event) => { event.preventDefault(); onSave(form, editing?.id); setForm(initial); setEditing(null); };
  const edit = (item) => { setEditing(item); setForm(decode(item)); window.scrollTo({ top: 0, behavior: "smooth" }); };
  return (
    <>
      <section className="section-head"><div><p>{kicker}</p><h1>{title}</h1><span>{description}</span></div></section>
      <section className="create-card">
        <div className="form-title"><h2>{editing ? "Редактирование" : "Создание"}</h2>{editing && <button className="secondary" type="button" onClick={() => { setEditing(null); setForm(initial); }}>Отменить</button>}</div>
        <form className="inline-form resource-form" onSubmit={submit}>
          {fields.map(([key, lbl, type = "text", options]) => (
            <label key={key}>{lbl}
              {type === "select" ? <select value={form[key]} onChange={(e) => setForm({ ...form, [key]: e.target.value })}>{options.map(([v, t]) => <option value={v} key={v}>{t}</option>)}</select> : <input type={type} required={key === "code" || key === "title" || key === "name"} value={form[key] ?? ""} onChange={(e) => setForm({ ...form, [key]: e.target.value })} />}
            </label>
          ))}
          <label className="check"><input type="checkbox" checked={form.is_active} onChange={(e) => setForm({ ...form, is_active: e.target.checked })} />Активен</label>
          <button>{editing ? "Сохранить" : "Создать"}</button>
        </form>
      </section>
      <section className="data-list">
        {items.length ? items.map((item) => (
          <article className="data-card editable" key={item.id} onClick={() => edit(item)}>
            {render(item)}
            <button className="secondary" onClick={(e) => { e.stopPropagation(); edit(item); }}>Изменить</button>
          </article>
        )) : <p className="empty">Пока ничего не создано.</p>}
      </section>
    </>
  );
}

function Integrations({ items, onToggle }) {
  return (
    <>
      <section className="section-head"><div><p>ДОСТУП / ПЛАТЕЖИ</p><h1>Платежные системы</h1><span>Включайте только настроенные провайдеры с заданными секретами.</span></div></section>
      <section className="provider-grid">
        {items.map((item) => (
          <article className="provider-card" key={item.provider_code}>
            <div><p>{providerNames[item.provider_code] || item.provider_code}</p><h2>{item.is_enabled ? "Включен" : "Выключен"}</h2><small>{item.is_configured ? "Учетные данные настроены" : "Не настроен"}</small></div>
            <button className={item.is_enabled ? "secondary" : "primary"} disabled={!item.is_configured} onClick={() => onToggle(item)}>{item.is_enabled ? "Выключить" : "Включить"}</button>
          </article>
        ))}
      </section>
    </>
  );
}

function Policies({ settings, referrals, onSave }) {
  return (
    <>
      <section className="section-head"><div><p>НАСТРОЙКИ / КОММЕРЦИЯ</p><h1>Правила</h1><span>Значения применяются к новым сценариям покупки и пробного доступа.</span></div></section>
      <section className="two-panel">
        <SettingsForm title="Пробный доступ" value={{ duration_days: settings.duration_seconds / 86400, traffic_gb: settings.traffic_bytes / 1024 ** 3 }} fields={["duration_days", "traffic_gb"]} labels={{ duration_days: "Срок, дней", traffic_gb: "Трафик, ГБ" }} onSave={(value) => onSave("/admin/trial-settings", { duration_seconds: Number(value.duration_days) * 86400, traffic_bytes: Number(value.traffic_gb) * 1024 ** 3 }, "Настройки пробного доступа сохранены.")} />
        <SettingsForm title="Реферальная программа" value={referrals} fields={["percent"]} labels={{ percent: "Вознаграждение, %" }} onSave={(value) => onSave("/admin/referral-settings", { percent: Number(value.percent) }, "Реферальное правило сохранено.")} />
      </section>
    </>
  );
}

function Runtime({ data, onSave }) {
  return (
    <>
      <section className="section-head"><div><p>НАСТРОЙКИ / СРЕДА</p><h1>Публичные адреса и транспорт</h1><span>Nginx, DNS и TLS остаются под ручным управлением на хосте.</span></div></section>
      <section className="two-panel">
        <SettingsForm title="Публичные адреса" value={{ ...data.publicSettings, cors_origins: data.publicSettings.cors_origins.join(", ") }} fields={["mini_app_url", "admin_url", "subscription_public_url", "telegram_webhook_url", "cors_origins", "support_url"]} labels={{ mini_app_url: "Mini App URL", admin_url: "Админка URL", subscription_public_url: "URL подписки", telegram_webhook_url: "Webhook Telegram", cors_origins: "CORS origins", support_url: "Поддержка" }} onSave={(value) => onSave("/admin/public-settings", { ...value, cors_origins: value.cors_origins.split(",").map((i) => i.trim()).filter(Boolean), support_url: value.support_url || null }, "Публичные адреса сохранены.")} />
        <SettingsForm title="Локальные порты" value={data.runtimeSettings} fields={["api_host_port", "mini_app_host_port", "admin_host_port", "telegram_webhook_host_port"]} labels={{ api_host_port: "API", mini_app_host_port: "Mini App", admin_host_port: "Админка", telegram_webhook_host_port: "Telegram webhook" }} onSave={(value) => onSave("/admin/runtime-settings", numberFields(value), "План портов сохранен. Перезапустите Compose и обновите nginx.")} />
      </section>
      <section className="two-panel">
        <SettingsForm title="Транспорт Telegram" value={data.telegramTransport} fields={["mode"]} options={{ mode: [["polling", "Polling"], ["webhook", "Webhook"]] }} labels={{ mode: "Режим" }} onSave={(value) => onSave("/admin/telegram-transport", value, "Транспорт Telegram сохранен. Перезапустите бота.")} />
        <article className="panel"><p>ГРАНИЦА ХОСТА</p><h2>Reverse proxy вне приложения</h2><p className="helper">Панель хранит URL и рекомендуемые порты, но не изменяет конфигурацию nginx, сертификаты или DNS.</p></article>
      </section>
    </>
  );
}

function SettingsForm({ title, value, fields, labels = {}, options, onSave }) {
  const [form, setForm] = useState(value);
  useEffect(() => setForm(value), [value]);
  return (
    <form className="form-card" onSubmit={(e) => { e.preventDefault(); onSave(form); }}>
      <div className="form-head"><div><p>КОНФИГУРАЦИЯ</p><h2>{title}</h2></div></div>
      {fields.map((field) => (
        <label key={field}>{labels[field] || field}
          {options?.[field] ? <select value={form[field] || ""} onChange={(e) => setForm({ ...form, [field]: e.target.value })}>{options[field].map(([v, t]) => <option key={v} value={v}>{t}</option>)}</select> : <input value={form[field] ?? ""} onChange={(e) => setForm({ ...form, [field]: e.target.value })} />}
        </label>
      ))}
      <div className="form-actions"><button className="primary">Сохранить</button></div>
    </form>
  );
}

function Audit({ data }) {
  return (
    <>
      <section className="section-head"><div><p>НАСТРОЙКИ / АУДИТ</p><h1>Журнал аудита</h1><span>Последние привилегированные изменения в панели управления.</span></div></section>
      <DataList data={data} render={(item) => (
        <><div><p>{item.target_type}</p><h2>{item.action}</h2><small>{date(item.created_at)}</small></div><b className="badge">{item.actor_user_id ? "админ" : "система"}</b></>
      )} />
    </>
  );
}

function DataList({ data, render }) {
  return (
    <>
      <section className="data-list">
        {data.items.length ? data.items.map((item) => <article className="data-card" key={item.id}>{render(item)}</article>) : <p className="empty">Нет записей.</p>}
      </section>
      <div className="pager"><span>{data.total ? `${data.offset + 1}-${Math.min(data.offset + data.limit, data.total)} из ${data.total}` : "0 записей"}</span></div>
    </>
  );
}

const numberFields = (value) => Object.fromEntries(Object.entries(value).map(([k, v]) => [k, /^\d+$/.test(String(v)) ? Number(v) : v]));

createRoot(document.getElementById("app")).render(<StrictMode><App /></StrictMode>);
