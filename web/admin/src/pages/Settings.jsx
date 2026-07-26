import { useEffect, useRef, useState } from "react";
import { useQueries, useMutation, useQuery } from "../lib/api.jsx";
import { providerLabel } from "../lib/format.js";
import { Badge, Card, ErrorState, Field, Loader, PageHeader, Toggle } from "../components/ui.jsx";

export function IntegrationsPage({ notify }) {
  const { data, error, loading, reload } = useQueries([
    ["providers", "/admin/payment-providers"],
    ["transport", "/admin/telegram-transport"],
    ["runtime", "/admin/runtime-settings"],
    ["publicSettings", "/admin/public-settings"],
  ]);
  const { run } = useMutation();
  const [restarting, setRestarting] = useState(false);

  if (loading && !data.providers) return <Loader title="Проверяем интеграции" />;
  if (error && !data.providers) return <ErrorState detail={error} onRetry={reload} />;

  const providers = data.providers || [];
  const transport = data.transport || {};
  const runtime = data.runtime || {};
  const publicSettings = data.publicSettings || {};

  const toggle = async (item) => {
    try {
      await run(`/admin/payment-providers/${item.provider_code}`, {
        method: "PUT",
        body: { is_enabled: !item.is_enabled },
      });
      notify({ message: item.is_enabled ? "Провайдер выключен." : "Провайдер включён." });
      reload();
    } catch (cause) {
      notify({ message: cause.message, tone: "error" });
    }
  };

  const restartBot = async () => {
    setRestarting(true);
    try {
      const result = await run("/admin/bot/restart", { method: "POST" });
      notify({ message: result?.message || "Сигнал перезапуска отправлен. Бот перезапустится автоматически." });
    } catch (cause) {
      notify({ message: cause.message, tone: "error" });
    } finally {
      setRestarting(false);
    }
  };

  return (
    <>
      <PageHeader
        kicker="Доступ / Интеграции"
        title="Интеграции"
        description="Состояние внешних сервисов и доступность платёжных провайдеров."
      />
      <div className="grid three">
        <Card title="Telegram Bot" action={<Badge tone="ok">активен</Badge>}>
          <p className="stat-big">{transport.mode === "webhook" ? "Webhook" : "Polling"}</p>
          <p className="muted-text">
            Токен задаётся секретом <code>TELEGRAM_BOT_TOKEN</code>. Режим меняется в разделе «Среда».
          </p>
          <div style={{ marginTop: "var(--space-3)" }}>
            <button
              type="button"
              className="btn ghost"
              disabled={restarting}
              onClick={restartBot}
            >
              {restarting ? "Перезапускаем..." : "Перезапустить бота"}
            </button>
          </div>
        </Card>
        <Card title="Remnawave" action={<Badge tone={publicSettings.subscription_public_url ? "ok" : "warn"}>{publicSettings.subscription_public_url ? "настроен" : "не задан"}</Badge>}>
          <p className="stat-big">VPN Backend</p>
          <p className="muted-text">Публичный URL подписки: {publicSettings.subscription_public_url || "—"}</p>
        </Card>
        <Card title="Шифрование" action={<Badge tone="ok">AES-256-GCM</Badge>}>
          <p className="stat-big">Ключ защищён</p>
          <p className="muted-text">
            <code>APPLICATION_ENCRYPTION_KEY</code> — 32 байта, хранится только в окружении.
          </p>
        </Card>
      </div>
      <div className="grid two">
        <Card title="Локальные порты" description="Публикуются только на 127.0.0.1">
          <ul className="stat-rows">
            <li>
              <span>API</span>
              <b className="num">{runtime.api_host_port ?? "—"}</b>
            </li>
            <li>
              <span>Mini App</span>
              <b className="num">{runtime.mini_app_host_port ?? "—"}</b>
            </li>
            <li>
              <span>Админка</span>
              <b className="num">{runtime.admin_host_port ?? "—"}</b>
            </li>
            <li>
              <span>Telegram webhook</span>
              <b className="num">{runtime.telegram_webhook_host_port ?? "—"}</b>
            </li>
          </ul>
        </Card>
        <Card title="Публичные адреса" description="Reverse proxy настраивается на хосте">
          <ul className="stat-rows">
            <li>
              <span>Mini App</span>
              <b>{publicSettings.mini_app_url || "—"}</b>
            </li>
            <li>
              <span>Админка</span>
              <b>{publicSettings.admin_url || "—"}</b>
            </li>
            <li>
              <span>Webhook Telegram</span>
              <b>{publicSettings.telegram_webhook_url || "—"}</b>
            </li>
            <li>
              <span>CORS origins</span>
              <b>{(publicSettings.cors_origins || []).length || 0}</b>
            </li>
          </ul>
        </Card>
      </div>
      <Card title="Платёжные провайдеры" description="Включайте только те адаптеры, для которых заданы секреты.">
        <div className="grid three">
          {providers.map((item) => (
            <article className="provider" key={item.provider_code}>
              <div>
                <p className="eyebrow">{providerLabel(item.provider_code)}</p>
                <strong>{item.is_enabled ? "Включён" : "Выключен"}</strong>
                <small>{item.is_configured ? "учётные данные заданы" : "секреты не настроены"}</small>
              </div>
              <button
                type="button"
                className={item.is_enabled ? "btn ghost" : "btn primary"}
                disabled={!item.is_configured}
                onClick={() => toggle(item)}
              >
                {item.is_enabled ? "Выключить" : "Включить"}
              </button>
            </article>
          ))}
        </div>
      </Card>
    </>
  );
}

/** Settings form that resets its local draft when the server payload changes. */
function SettingsForm({ id, title, description, value, children, onSubmit, pending }) {
  return (
    <Card title={title} description={description}>
      <form id={id} className="form-grid" onSubmit={onSubmit}>
        {children}
        <div className="form-actions">
          <button type="submit" className="btn primary" disabled={pending}>
            {pending ? "Сохраняем..." : "Сохранить"}
          </button>
        </div>
      </form>
    </Card>
  );
}

const useDraft = (value) => {
  const [draft, setDraft] = useState(value);
  const prev = useRef(JSON.stringify(value));
  useEffect(() => {
    const next = JSON.stringify(value);
    if (prev.current !== next) {
      prev.current = next;
      setDraft(value);
    }
  }, [value]);
  return [draft, setDraft];
};

export function PoliciesPage({ notify }) {
  const { data, error, loading, reload } = useQueries([
    ["trial", "/admin/trial-settings"],
    ["referral", "/admin/referral-settings"],
  ]);
  const { run, pending } = useMutation();
  const [trial, setTrial] = useDraft({
    duration_days: String((data.trial?.duration_seconds ?? 0) / 86400 || ""),
    traffic_gb: String((data.trial?.traffic_bytes ?? 0) / 1024 ** 3 || ""),
  });
  const [referral, setReferral] = useDraft({ percent: String(data.referral?.percent ?? "") });

  if (loading && !data.trial) return <Loader title="Загружаем правила" />;
  if (error && !data.trial) return <ErrorState detail={error} onRetry={reload} />;

  const save = async (path, body, message) => {
    try {
      await run(path, { method: "PUT", body });
      notify({ message });
      reload();
    } catch (cause) {
      notify({ message: cause.message, tone: "error" });
    }
  };

  return (
    <>
      <PageHeader
        kicker="Настройки / Коммерция"
        title="Правила"
        description="Применяются к новым покупкам и активациям пробного доступа."
      />
      <div className="grid two">
        <SettingsForm
          id="trial-form"
          title="Пробный доступ"
          description="Выдаётся один раз на клиента"
          pending={pending}
          onSubmit={(event) => {
            event.preventDefault();
            save(
              "/admin/trial-settings",
              {
                duration_seconds: Math.round(Number(trial.duration_days || 0) * 86400),
                traffic_bytes: Math.round(Number(trial.traffic_gb || 0) * 1024 ** 3),
              },
              "Настройки пробного доступа сохранены.",
            );
          }}
        >
          <Field label="Срок, дней">
            <input
              required
              type="number"
              min="1"
              value={trial.duration_days}
              onChange={(event) => setTrial({ ...trial, duration_days: event.target.value })}
            />
          </Field>
          <Field label="Трафик, ГБ">
            <input
              required
              type="number"
              min="1"
              value={trial.traffic_gb}
              onChange={(event) => setTrial({ ...trial, traffic_gb: event.target.value })}
            />
          </Field>
        </SettingsForm>
        <SettingsForm
          id="referral-form"
          title="Реферальная программа"
          description="Процент от оплаченного счёта приглашённого клиента"
          pending={pending}
          onSubmit={(event) => {
            event.preventDefault();
            save("/admin/referral-settings", { percent: Number(referral.percent || 0) }, "Реферальное правило сохранено.");
          }}
        >
          <Field label="Вознаграждение, %" hint="0 отключает начисления">
            <input
              required
              type="number"
              min="0"
              max="100"
              value={referral.percent}
              onChange={(event) => setReferral({ percent: event.target.value })}
            />
          </Field>
        </SettingsForm>
      </div>
    </>
  );
}

function normalizeDomain(value) {
  return value.replace(/^https?:\/\//, "").replace(/\/+$/, "");
}

function buildUrl(domain, path = "") {
  if (!domain) return "";
  return `https://${normalizeDomain(domain)}${path}`;
}

function extractDomain(url) {
  if (!url) return "";
  return normalizeDomain(url);
}

export function RuntimePage({ notify }) {
  const { data, error, loading, reload } = useQueries([
    ["publicSettings", "/admin/public-settings"],
    ["runtime", "/admin/runtime-settings"],
    ["transport", "/admin/telegram-transport"],
  ]);
  const { run, pending } = useMutation();
  const [restarting, setRestarting] = useState(false);

  const ps = data.publicSettings || {};
  const [baseDomain, setBaseDomain] = useDraft(extractDomain(ps.mini_app_url) || extractDomain(ps.admin_url) || "");
  const [webhookDomain, setWebhookDomain] = useDraft(extractDomain(ps.telegram_webhook_url) || "");
  const [supportUrl, setSupportUrl] = useDraft(ps.support_url ?? "");
  const [corsInput, setCorsInput] = useDraft((ps.cors_origins || []).join(", "));

  const mini_app_url = buildUrl(baseDomain);
  const admin_url = buildUrl(baseDomain ? `admin.${baseDomain}` : "");
  const subscription_public_url = buildUrl(baseDomain ? `sub.${baseDomain}` : "");
  const telegram_webhook_url = buildUrl(webhookDomain, "/telegram/webhook");

  const [ports, setPorts] = useDraft({
    api_host_port: String(data.runtime?.api_host_port ?? ""),
    mini_app_host_port: String(data.runtime?.mini_app_host_port ?? ""),
    admin_host_port: String(data.runtime?.admin_host_port ?? ""),
    telegram_webhook_host_port: String(data.runtime?.telegram_webhook_host_port ?? ""),
  });
  const [mode, setMode] = useDraft(data.transport?.mode ?? "polling");

  if (loading && !data.publicSettings) return <Loader title="Загружаем параметры среды" />;
  if (error && !data.publicSettings) return <ErrorState detail={error} onRetry={reload} />;

  const save = async (path, body, message) => {
    try {
      await run(path, { method: "PUT", body });
      notify({ message });
      reload();
    } catch (cause) {
      notify({ message: cause.message, tone: "error" });
    }
  };

  return (
    <>
      <PageHeader
        kicker="Настройки / Среда"
        title="Публичные адреса и транспорт"
        description="Nginx, DNS и TLS остаются под ручным управлением на хосте."
      />
      <div className="grid two">
        <SettingsForm
          id="addresses-form"
          title="Публичные адреса"
          description="Введите базовый домен — пути и поддомены подставятся автоматически"
          pending={pending}
          onSubmit={(event) => {
            event.preventDefault();
            save(
              "/admin/public-settings",
              {
                mini_app_url,
                admin_url,
                subscription_public_url,
                telegram_webhook_url,
                cors_origins: corsInput
                  .split(",")
                  .map((origin) => origin.trim())
                  .filter(Boolean),
                support_url: supportUrl || null,
              },
              "Публичные адреса сохранены.",
            );
          }}
        >
          <Field label="Домен" hint="app.vpn-test.zeragorn.xyz" wide>
            <input
              required
              value={baseDomain}
              onChange={(event) => setBaseDomain(event.target.value)}
              placeholder="app.vpn-test.zeragorn.xyz"
            />
          </Field>
          <div className="field-preview" style={{ fontSize: 12, opacity: 0.6, lineHeight: 1.6, marginBottom: 8 }}>
            Mini App: {mini_app_url}<br/>
            Админка: {admin_url}<br/>
            Подписки: {subscription_public_url}
          </div>
          <Field label="Домен Webhook" hint="tg.vpn-test.zeragorn.xyz" wide>
            <input
              required
              value={webhookDomain}
              onChange={(event) => setWebhookDomain(event.target.value)}
              placeholder="tg.vpn-test.zeragorn.xyz"
            />
          </Field>
          <div className="field-preview" style={{ fontSize: 12, opacity: 0.6, marginBottom: 8 }}>
            Webhook: {telegram_webhook_url}
          </div>
          <Field label="CORS origins" hint="Через запятую" wide>
            <input value={corsInput} onChange={(event) => setCorsInput(event.target.value)} />
          </Field>
          <Field label="Поддержка" hint="Необязательно" wide>
            <input value={supportUrl} onChange={(event) => setSupportUrl(event.target.value)} placeholder="https://t.me/..." />
          </Field>
        </SettingsForm>
        <div className="stack">
          <SettingsForm
            id="ports-form"
            title="Локальные порты"
            description="Только 127.0.0.1, требуется перезапуск Compose"
            pending={pending}
            onSubmit={(event) => {
              event.preventDefault();
              save(
                "/admin/runtime-settings",
                Object.fromEntries(Object.entries(ports).map(([key, value]) => [key, Number(value)])),
                "План портов сохранён. Перезапустите Compose и обновите nginx.",
              );
            }}
          >
            {[
              ["api_host_port", "API"],
              ["mini_app_host_port", "Mini App"],
              ["admin_host_port", "Админка"],
              ["telegram_webhook_host_port", "Telegram webhook"],
            ].map(([key, text]) => (
              <Field label={text} key={key}>
                <input
                  required
                  type="number"
                  min="1"
                  max="65535"
                  value={ports[key]}
                  onChange={(event) => setPorts({ ...ports, [key]: event.target.value })}
                />
              </Field>
            ))}
          </SettingsForm>
          <SettingsForm
            id="transport-form"
            title="Транспорт Telegram"
            description="После смены режима перезапустите бота"
            pending={pending}
            onSubmit={(event) => {
              event.preventDefault();
              save("/admin/telegram-transport", { mode }, "Транспорт Telegram сохранён. Перезапустите бота.");
            }}
          >
            <Field label="Режим">
              <select value={mode} onChange={(event) => setMode(event.target.value)}>
                <option value="polling">Polling</option>
                <option value="webhook">Webhook</option>
              </select>
            </Field>
          </SettingsForm>
          <Card title="Перезапуск бота" description="Перезапуск через API — бот завершит работу, Docker поднимет его автоматически">
            <button
              type="button"
              className="btn primary"
              disabled={restarting}
              onClick={async () => {
                setRestarting(true);
                try {
                  const result = await run("/admin/bot/restart", { method: "POST" });
                  notify({ message: result?.message || "Сигнал перезапуска отправлен." });
                } catch (cause) {
                  notify({ message: cause.message, tone: "error" });
                } finally {
                  setRestarting(false);
                }
              }}
            >
              {restarting ? "Перезапускаем..." : "Перезапустить бота"}
            </button>
          </Card>
        </div>
      </div>
    </>
  );
}

export function SecretsPage({ notify }) {
  const { data, error, loading, reload } = useQuery("/admin/secrets", { optional: true });
  const { run } = useMutation();
  const [drafts, setDrafts] = useState({});
  const [revealed, setRevealed] = useState({});
  const [saving, setSaving] = useState(null);

  const items = data?.items || [];

  const save = async (key) => {
    setSaving(key);
    try {
      await run("/admin/secrets", { method: "PUT", body: { key, value: drafts[key] } });
      notify({ message: `Секрет ${key} обновлён. Требуется перезапуск API.` });
      setDrafts((prev) => {
        const next = { ...prev };
        delete next[key];
        return next;
      });
      reload();
    } catch (cause) {
      notify({ message: cause.message, tone: "error" });
    } finally {
      setSaving(null);
    }
  };

  return (
    <>
      <PageHeader
        kicker="Настройки / Секреты"
        title="Управление секретами"
        description="Значения хранятся зашифрованными и никогда не возвращаются в интерфейс."
      />
      {loading && !items.length ? (
        <Loader title="Загружаем список секретов" />
      ) : error ? (
        <ErrorState detail={error} onRetry={reload} />
      ) : !items.length ? (
        <Card title="Секреты недоступны">
          <p className="muted-text">
            Эндпоинт <code>/admin/secrets</code> вернул 404. Обновите образ API до текущего релиза.
          </p>
        </Card>
      ) : (
        <div className="grid two">
          {items.map((item) => (
            <Card
              key={item.key}
              title={item.label}
              description={item.description}
              action={<Badge tone={item.is_set ? "ok" : "warn"}>{item.is_set ? "задан" : "не задан"}</Badge>}
            >
              <div className="secret-row">
                <div className="secret-input">
                  <input
                    type={revealed[item.key] ? "text" : "password"}
                    autoComplete="off"
                    placeholder={item.is_set ? "••••••••" : "Введите значение"}
                    aria-label={item.label}
                    value={drafts[item.key] ?? ""}
                    onChange={(event) => setDrafts((prev) => ({ ...prev, [item.key]: event.target.value }))}
                  />
                  <button
                    type="button"
                    onClick={() => setRevealed((prev) => ({ ...prev, [item.key]: !prev[item.key] }))}
                    aria-label={revealed[item.key] ? "Скрыть значение" : "Показать значение"}
                  >
                    {revealed[item.key] ? "скрыть" : "показать"}
                  </button>
                </div>
                <button
                  type="button"
                  className="btn primary"
                  disabled={!drafts[item.key] || saving === item.key}
                  onClick={() => save(item.key)}
                >
                  {saving === item.key ? "..." : "Заменить"}
                </button>
              </div>
            </Card>
          ))}
        </div>
      )}
    </>
  );
}
