import { useState } from "react";
import { useQuery, useMutation } from "../lib/api.jsx";
import { count, date, dateTime, label, money, statusTone, timeAgo, userLabel } from "../lib/format.js";
import {
  Badge,
  Card,
  DataTable,
  ErrorState,
  Field,
  Modal,
  PageHeader,
  SearchInput,
  Select,
  SegmentedControl,
} from "../components/ui.jsx";

const PAGE_SIZE = 25;

/** Shared hook for the list pages: keeps offset/search/filter in one place. */
function useListPage(path, extraParams = {}) {
  const [offset, setOffset] = useState(0);
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState("");
  const query = useQuery(path, {
    params: { limit: PAGE_SIZE, offset, q: search || undefined, ...extraParams, ...(filter ? { status: filter } : {}) },
  });
  return {
    ...query,
    offset,
    search,
    filter,
    setOffset,
    onSearch: (value) => {
      setSearch(value);
      setOffset(0);
    },
    onFilter: (value) => {
      setFilter(value);
      setOffset(0);
    },
  };
}

export function UsersPage({ notify }) {
  const list = useListPage("/admin/users");
  const { run, pending } = useMutation();
  const [open, setOpen] = useState(false);
  const [form, setForm] = useState({ telegram_user_id: "", username: "", first_name: "", language_code: "ru" });

  const submit = async (event) => {
    event.preventDefault();
    try {
      await run("/admin/users", {
        method: "POST",
        body: {
          telegram_user_id: Number(form.telegram_user_id),
          username: form.username.replace(/^@/, "") || null,
          first_name: form.first_name,
          language_code: form.language_code,
        },
      });
      notify({ message: "Клиент создан." });
      setOpen(false);
      setForm({ telegram_user_id: "", username: "", first_name: "", language_code: "ru" });
      list.reload();
    } catch (error) {
      notify({ message: error.message, tone: "error" });
    }
  };

  return (
    <>
      <PageHeader
        kicker="Главное / Клиенты"
        title="Клиенты"
        description="Профили создаются автоматически после первого входа через Telegram."
        actions={
          <button type="button" className="btn primary" onClick={() => setOpen(true)}>
            Добавить клиента
          </button>
        }
      />
      <Card
        title={`Всего ${count(list.data?.total ?? 0)}`}
        description="Поиск по username и Telegram ID"
        action={<SearchInput value={list.search} onChange={list.onSearch} placeholder="username или ID" />}
        padded={false}
      >
        {list.error ? (
          <ErrorState detail={list.error} onRetry={list.reload} />
        ) : (
          <DataTable
            columns={[
              {
                key: "user",
                title: "Клиент",
                render: (row) => (
                  <div className="cell-user">
                    <span className="avatar">{(row.first_name || "?").charAt(0).toUpperCase()}</span>
                    <span>
                      <b>{userLabel(row)}</b>
                      <small>ID {row.telegram_user_id}</small>
                    </span>
                  </div>
                ),
              },
              { key: "lang", title: "Язык", render: (row) => <Badge>{row.language_code}</Badge>, width: "90px" },
              {
                key: "balance",
                title: "Баланс",
                align: "right",
                render: (row) => <b className="num">{money(row.balance_minor, row.currency_code)}</b>,
              },
              { key: "created", title: "Регистрация", align: "right", render: (row) => date(row.created_at), width: "150px" },
            ]}
            rows={list.data?.items || []}
            keyOf={(row) => row.id}
            page={list.data}
            onPageChange={list.setOffset}
            loading={list.loading}
            emptyTitle="Клиентов пока нет"
            emptyDetail="Первый профиль появится после входа пользователя в Mini App."
          />
        )}
      </Card>
      {open && (
        <Modal
          title="Новый клиент"
          onClose={() => setOpen(false)}
          footer={
            <>
              <button type="button" className="btn ghost" onClick={() => setOpen(false)}>
                Отмена
              </button>
              <button type="submit" form="user-form" className="btn primary" disabled={pending}>
                {pending ? "Создаём..." : "Создать"}
              </button>
            </>
          }
        >
          <form id="user-form" className="form-grid" onSubmit={submit}>
            <Field label="Telegram ID">
              <input
                required
                inputMode="numeric"
                pattern="[0-9]+"
                placeholder="123456789"
                value={form.telegram_user_id}
                onChange={(event) => setForm({ ...form, telegram_user_id: event.target.value })}
              />
            </Field>
            <Field label="Username" hint="Необязательно">
              <input placeholder="@username" value={form.username} onChange={(event) => setForm({ ...form, username: event.target.value })} />
            </Field>
            <Field label="Имя">
              <input required value={form.first_name} onChange={(event) => setForm({ ...form, first_name: event.target.value })} />
            </Field>
            <Field label="Язык">
              <select value={form.language_code} onChange={(event) => setForm({ ...form, language_code: event.target.value })}>
                <option value="ru">Русский</option>
                <option value="en">English</option>
              </select>
            </Field>
          </form>
        </Modal>
      )}
    </>
  );
}

const INVOICE_STATUSES = [
  ["", "Все статусы"],
  ["paid", "Оплачены"],
  ["pending", "Ожидают"],
  ["expired", "Истекли"],
  ["cancelled", "Отменены"],
];

const PROVIDERS = [
  ["", "Все провайдеры"],
  ["crypto_pay", "Crypto Pay"],
  ["anore", "Anore"],
  ["telegram_stars", "Telegram Stars"],
];

export function InvoicesPage() {
  const [provider, setProvider] = useState("");
  const list = useListPage("/admin/invoices", provider ? { provider } : {});
  return (
    <>
      <PageHeader kicker="Операции / Платежи" title="Платежи" description="Счета, статусы и распределение по провайдерам." />
      <Card
        title={`Найдено ${count(list.data?.total ?? 0)}`}
        action={
          <div className="toolbar">
            <SearchInput value={list.search} onChange={list.onSearch} placeholder="username или ID" />
            <Select value={list.filter} options={INVOICE_STATUSES} onChange={list.onFilter} ariaLabel="Статус счёта" />
            <Select
              value={provider}
              options={PROVIDERS}
              onChange={(value) => {
                setProvider(value);
                list.setOffset(0);
              }}
              ariaLabel="Платёжный провайдер"
            />
          </div>
        }
        padded={false}
      >
        {list.error ? (
          <ErrorState detail={list.error} onRetry={list.reload} />
        ) : (
          <DataTable
            columns={[
              { key: "user", title: "Клиент", render: (row) => <b>{userLabel(row)}</b> },
              { key: "provider", title: "Провайдер", render: (row) => label(row.provider) },
              { key: "purpose", title: "Назначение", render: (row) => label(row.purpose) },
              {
                key: "amount",
                title: "Сумма",
                align: "right",
                render: (row) => <b className="num">{money(row.amount_minor, row.currency_code)}</b>,
              },
              {
                key: "status",
                title: "Статус",
                render: (row) => <Badge tone={statusTone(row.status)}>{label(row.status)}</Badge>,
                width: "130px",
              },
              {
                key: "created",
                title: "Создан / оплачен",
                align: "right",
                render: (row) => (
                  <span className="cell-stack">
                    <span>{dateTime(row.created_at)}</span>
                    <small>{row.paid_at ? dateTime(row.paid_at) : "—"}</small>
                  </span>
                ),
                width: "170px",
              },
            ]}
            rows={list.data?.items || []}
            keyOf={(row) => row.id}
            page={list.data}
            onPageChange={list.setOffset}
            loading={list.loading}
            emptyTitle="Платежей нет"
            emptyDetail="Счета появятся после первой оплаты через Mini App или бота."
          />
        )}
      </Card>
    </>
  );
}

const SUBSCRIPTION_STATUSES = [
  ["", "Все статусы"],
  ["active", "Активные"],
  ["provisioning_pending", "В обработке"],
  ["suspended", "Приостановлены"],
  ["expired", "Истекли"],
  ["failed", "С ошибкой"],
];

export function SubscriptionsPage() {
  const list = useListPage("/admin/subscriptions");
  return (
    <>
      <PageHeader kicker="Операции / Доступ" title="Подписки" description="Жизненный цикл доступа и состояние выдачи конфигурации." />
      <Card
        title={`Найдено ${count(list.data?.total ?? 0)}`}
        action={
          <div className="toolbar">
            <SearchInput value={list.search} onChange={list.onSearch} placeholder="username, тариф или ID" />
            <Select value={list.filter} options={SUBSCRIPTION_STATUSES} onChange={list.onFilter} ariaLabel="Статус подписки" />
          </div>
        }
        padded={false}
      >
        {list.error ? (
          <ErrorState detail={list.error} onRetry={list.reload} />
        ) : (
          <DataTable
            columns={[
              { key: "user", title: "Клиент", render: (row) => <b>{userLabel(row)}</b> },
              {
                key: "tariff",
                title: "Тариф",
                render: (row) => (row.is_trial ? <Badge>пробный</Badge> : row.tariff_code || "—"),
              },
              {
                key: "status",
                title: "Статус",
                render: (row) => <Badge tone={statusTone(row.status)}>{label(row.status)}</Badge>,
                width: "130px",
              },
              { key: "starts", title: "Начало", align: "right", render: (row) => date(row.starts_at), width: "130px" },
              {
                key: "expires",
                title: "Действует до",
                align: "right",
                render: (row) => (
                  <span className="cell-stack">
                    <span>{date(row.expires_at)}</span>
                    <small>{row.expires_at ? timeAgo(row.expires_at) : ""}</small>
                  </span>
                ),
                width: "150px",
              },
            ]}
            rows={list.data?.items || []}
            keyOf={(row) => row.id}
            page={list.data}
            onPageChange={list.setOffset}
            loading={list.loading}
            emptyTitle="Подписок нет"
            emptyDetail="Подписки создаются после оплаты или активации пробного доступа."
          />
        )}
      </Card>
    </>
  );
}

const AUDIT_SCOPES = [
  ["", "Все"],
  ["tariff", "Тарифы"],
  ["promo", "Промокоды"],
  ["settings", "Настройки"],
  ["secret", "Секреты"],
  ["user", "Клиенты"],
];

export function AuditPage() {
  const [offset, setOffset] = useState(0);
  const [action, setAction] = useState("");
  const { data, error, loading, reload } = useQuery("/admin/audit", {
    params: { limit: PAGE_SIZE, offset, action: action || undefined },
  });
  return (
    <>
      <PageHeader kicker="Настройки / Аудит" title="Журнал аудита" description="Привилегированные изменения с актором и объектом." />
      <Card
        title={`Записей ${count(data?.total ?? 0)}`}
        action={
          <SegmentedControl
            value={action}
            options={AUDIT_SCOPES}
            onChange={(value) => {
              setAction(value);
              setOffset(0);
            }}
            ariaLabel="Фильтр по действию"
          />
        }
        padded={false}
      >
        {error ? (
          <ErrorState detail={error} onRetry={reload} />
        ) : (
          <DataTable
            columns={[
              { key: "action", title: "Действие", render: (row) => <b>{row.action}</b> },
              { key: "target", title: "Объект", render: (row) => row.target_type },
              {
                key: "actor",
                title: "Актор",
                render: (row) => <Badge tone={row.actor_user_id ? "ok" : "warn"}>{row.actor_user_id ? "админ" : "система"}</Badge>,
                width: "120px",
              },
              { key: "created", title: "Время", align: "right", render: (row) => dateTime(row.created_at), width: "170px" },
            ]}
            rows={data?.items || []}
            keyOf={(row) => row.id}
            page={data}
            onPageChange={setOffset}
            loading={loading}
            emptyTitle="Журнал пуст"
            emptyDetail="Записи появятся после первого изменения в панели."
          />
        )}
      </Card>
    </>
  );
}
