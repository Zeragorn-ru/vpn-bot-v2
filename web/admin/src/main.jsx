import { StrictMode, useCallback, useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { ApiProvider, clearSession, readSession, request, writeSession } from "./lib/api.jsx";
import { CommandPalette, Loader, Toast } from "./components/ui.jsx";
import Login from "./pages/Login.jsx";
import Overview from "./pages/Overview.jsx";
import { AuditPage, InvoicesPage, SubscriptionsPage, UsersPage } from "./pages/Records.jsx";
import { ChannelsPage, PromosPage, TariffsPage } from "./pages/Catalog.jsx";
import { IntegrationsPage, PoliciesPage, RuntimePage, SecretsPage } from "./pages/Settings.jsx";
import "../styles.css";

const NAV = [
  [
    "Главное",
    [
      ["overview", "Обзор", "▤", "дашборд метрики выручка"],
      ["users", "Клиенты", "◍", "пользователи баланс"],
    ],
  ],
  [
    "Операции",
    [
      ["payments", "Платежи", "₽", "счета инвойсы провайдеры"],
      ["subscriptions", "Подписки", "◈", "доступ выдача"],
      ["tariffs", "Тарифы", "▦", "каталог цены"],
      ["promos", "Промокоды", "%", "скидки маркетинг"],
    ],
  ],
  [
    "Доступ",
    [
      ["channels", "Каналы", "◎", "telegram обязательная подписка"],
      ["integrations", "Интеграции", "⇄", "провайдеры remnawave"],
    ],
  ],
  [
    "Настройки",
    [
      ["policies", "Правила", "⚙", "триал рефералы"],
      ["runtime", "Среда", "▣", "порты адреса webhook"],
      ["secrets", "Секреты", "◆", "токены ключи"],
      ["audit", "Журнал", "≡", "аудит история"],
    ],
  ],
];

const FLAT_NAV = NAV.flatMap(([, items]) => items);
const PRESETS = [
  ["frost", "#8fa3ae"],
  ["matrix", "#35d39a"],
  ["cyan", "#22d3ee"],
  ["violet", "#a78bfa"],
  ["amber", "#f0b429"],
  ["red", "#ef5350"],
];
const DENSITIES = [
  ["compact", "Компактный"],
  ["comfortable", "Комфортный"],
  ["spacious", "Просторный"],
];

const readHashTab = () => {
  const id = window.location.hash.replace(/^#\/?/, "");
  return FLAT_NAV.some(([candidate]) => candidate === id) ? id : "overview";
};

function Appearance({ preset, setPreset, density, setDensity, onClose }) {
  useEffect(() => {
    const handler = (event) => {
      if (!event.target.closest(".popover") && !event.target.closest("[data-appearance-trigger]")) onClose();
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [onClose]);
  return (
    <div className="popover" role="dialog" aria-label="Оформление">
      <p className="eyebrow">Акцент</p>
      <div className="swatches">
        {PRESETS.map(([id, color]) => (
          <button
            type="button"
            key={id}
            className={preset === id ? "swatch active" : "swatch"}
            style={{ background: color }}
            onClick={() => setPreset(id)}
            aria-label={`Тема ${id}`}
            aria-pressed={preset === id}
          />
        ))}
      </div>
      <p className="eyebrow">Плотность</p>
      <select value={density} onChange={(event) => setDensity(event.target.value)} aria-label="Плотность интерфейса">
        {DENSITIES.map(([id, text]) => (
          <option value={id} key={id}>
            {text}
          </option>
        ))}
      </select>
    </div>
  );
}

function Shell({ token, onLogout }) {
  const [tab, setTab] = useState(readHashTab);
  const [notice, setNotice] = useState(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [appearanceOpen, setAppearanceOpen] = useState(false);
  const [navOpen, setNavOpen] = useState(false);
  const [preset, setPreset] = useState(() => localStorage.getItem("vpn-preset") || "frost");
  const [density, setDensity] = useState(() => localStorage.getItem("vpn-density") || "comfortable");

  useEffect(() => {
    document.documentElement.dataset.preset = preset;
    localStorage.setItem("vpn-preset", preset);
  }, [preset]);
  useEffect(() => {
    document.documentElement.dataset.density = density;
    localStorage.setItem("vpn-density", density);
  }, [density]);
  useEffect(() => {
    const sync = () => setTab(readHashTab());
    window.addEventListener("hashchange", sync);
    return () => window.removeEventListener("hashchange", sync);
  }, []);
  useEffect(() => {
    const handler = (event) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen((open) => !open);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  const navigate = useCallback((id) => {
    window.location.hash = `#/${id}`;
    setTab(id);
    setPaletteOpen(false);
    setNavOpen(false);
  }, []);

  const notify = useCallback((value) => setNotice(value), []);

  const pages = {
    overview: <Overview />,
    users: <UsersPage notify={notify} />,
    payments: <InvoicesPage />,
    subscriptions: <SubscriptionsPage />,
    tariffs: <TariffsPage notify={notify} />,
    promos: <PromosPage notify={notify} />,
    channels: <ChannelsPage notify={notify} />,
    integrations: <IntegrationsPage notify={notify} />,
    policies: <PoliciesPage notify={notify} />,
    runtime: <RuntimePage notify={notify} />,
    secrets: <SecretsPage notify={notify} />,
    audit: <AuditPage />,
  };

  const activeTitle = FLAT_NAV.find(([id]) => id === tab)?.[1] ?? "Обзор";

  return (
    <div className="shell">
      <a className="skip-link" href="#main">
        Перейти к содержимому
      </a>
      <aside className={navOpen ? "sidebar open" : "sidebar"}>
        <div className="sidebar-brand">
          <b aria-hidden="true" />
          <span>
            VPN <small>OPS</small>
          </span>
        </div>
        <nav className="sidebar-nav" aria-label="Разделы панели">
          {NAV.map(([group, items]) => (
            <div className="nav-group" key={group}>
              <p className="eyebrow">{group}</p>
              {items.map(([id, title, icon]) => (
                <button
                  type="button"
                  key={id}
                  className={tab === id ? "nav-item active" : "nav-item"}
                  onClick={() => navigate(id)}
                  aria-current={tab === id ? "page" : undefined}
                >
                  <i aria-hidden="true">{icon}</i>
                  {title}
                </button>
              ))}
            </div>
          ))}
        </nav>
        <div className="sidebar-foot">
          <span>Все изменения фиксируются в журнале аудита</span>
        </div>
      </aside>
      {navOpen && <div className="nav-backdrop" onClick={() => setNavOpen(false)} role="presentation" />}
      <div className="main">
        <header className="topbar">
          <div className="topbar-left">
            <button type="button" className="btn icon only-mobile" onClick={() => setNavOpen(true)} aria-label="Открыть меню">
              ≡
            </button>
            <button type="button" className="palette-trigger" onClick={() => setPaletteOpen(true)}>
              <span aria-hidden="true">⌕</span>
              <span className="palette-trigger-text">Быстрый переход</span>
              <kbd>⌘K</kbd>
            </button>
          </div>
          <div className="topbar-right">
            <span className="session-dot">сессия активна</span>
            <div className="popover-anchor">
              <button
                type="button"
                className="btn icon"
                data-appearance-trigger
                onClick={() => setAppearanceOpen((open) => !open)}
                aria-label="Оформление"
                aria-expanded={appearanceOpen}
              >
                ◐
              </button>
              {appearanceOpen && (
                <Appearance
                  preset={preset}
                  setPreset={setPreset}
                  density={density}
                  setDensity={setDensity}
                  onClose={() => setAppearanceOpen(false)}
                />
              )}
            </div>
            <button type="button" className="btn ghost" onClick={onLogout}>
              Выйти
            </button>
          </div>
        </header>
        <main className="content" id="main" aria-label={activeTitle}>
          {pages[tab]}
        </main>
      </div>
      {paletteOpen && <CommandPalette items={FLAT_NAV} onSelect={navigate} onClose={() => setPaletteOpen(false)} />}
      <Toast notice={notice} onDismiss={() => setNotice(null)} />
    </div>
  );
}

function App() {
  const [token, setToken] = useState(() => readSession()?.token || null);
  const [setupRequired, setSetupRequired] = useState(false);
  const [booting, setBooting] = useState(!readSession());
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (token) return;
    let cancelled = false;
    setBooting(true);
    request("/admin/setup-status")
      .then((status) => {
        if (!cancelled) setSetupRequired(Boolean(status.setup_required));
      })
      .catch((cause) => {
        if (!cancelled) setError(cause.message);
      })
      .finally(() => {
        if (!cancelled) setBooting(false);
      });
    return () => {
      cancelled = true;
    };
  }, [token]);

  const authenticate = async ({ login, password }) => {
    setPending(true);
    setError("");
    try {
      const auth = await request(setupRequired ? "/admin/setup" : "/admin/login", {
        method: "POST",
        body: { login, password },
      });
      writeSession(auth.access_token, auth.expires_at);
      setToken(auth.access_token);
    } catch (cause) {
      setError(cause.message);
    } finally {
      setPending(false);
    }
  };

  const logout = useCallback(() => {
    clearSession();
    setToken(null);
    setError("");
  }, []);

  const onUnauthorized = useCallback(() => {
    clearSession();
    setToken(null);
    setError("Сессия истекла. Войдите заново.");
  }, []);

  const provider = useMemo(() => ({ token, onUnauthorized }), [token, onUnauthorized]);

  if (booting) return <Loader title="Загружаем панель управления" />;
  if (!token) return <Login setupRequired={setupRequired} error={error} pending={pending} onSubmit={authenticate} />;

  return (
    <ApiProvider token={provider.token} onUnauthorized={provider.onUnauthorized}>
      <Shell token={token} onLogout={logout} />
    </ApiProvider>
  );
}

createRoot(document.getElementById("app")).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
