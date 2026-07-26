import { useState } from "react";
import { useQuery, useMutation } from "../lib/api.jsx";
import { bytes, count, duration, label, money } from "../lib/format.js";
import {
  Badge,
  Card,
  DataTable,
  ErrorState,
  Field,
  Modal,
  PageHeader,
  Toggle,
} from "../components/ui.jsx";

const EMPTY_TARIFF = {
  code: "",
  name_ru: "",
  name_en: "",
  description_ru: "",
  duration_days: "30",
  traffic_gb: "",
  position: "0",
  amount_rub: "",
  currency_code: "RUB",
  is_active: true,
};

const decodeTariff = (item) => ({
  code: item.code,
  name_ru: item.name?.ru || "",
  name_en: item.name?.en || "",
  description_ru: item.description?.ru || "",
  duration_days: item.duration_seconds ? String(item.duration_seconds / 86400) : "",
  traffic_gb: item.traffic_bytes == null ? "" : String(Math.round(item.traffic_bytes / 1024 ** 3)),
  position: String(item.position ?? 0),
  amount_rub: String((item.amount_minor ?? 0) / 100),
  currency_code: item.currency_code || "RUB",
  is_active: item.is_active,
});

const encodeTariff = (form) => ({
  code: form.code.trim(),
  name: { ru: form.name_ru, ...(form.name_en ? { en: form.name_en } : {}) },
  description: form.description_ru ? { ru: form.description_ru } : {},
  duration_seconds: form.duration_days === "" ? null : Number(form.duration_days) * 86400,
  traffic_bytes: form.traffic_gb === "" ? null : Math.round(Number(form.traffic_gb) * 1024 ** 3),
  position: Number(form.position || 0),
  is_active: form.is_active,
  amount_minor: Math.round(Number(form.amount_rub || 0) * 100),
  currency_code: form.currency_code || "RUB",
});

export function TariffsPage({ notify }) {
  const { data, error, loading, reload } = useQuery("/admin/tariffs");
  const { run, pending } = useMutation();
  const [editing, setEditing] = useState(null);

  const save = async (event) => {
    event.preventDefault();
    const isUpdate = Boolean(editing?.id);
    try {
      await run(`/admin/tariffs${isUpdate ? `/${editing.id}` : ""}`, {
        method: isUpdate ? "PUT" : "POST",
        body: encodeTariff(editing),
      });
      notify({ message: isUpdate ? "Тариф обновлён." : "Тариф создан." });
      setEditing(null);
      reload();
    } catch (cause) {
      notify({ message: cause.message, tone: "error" });
    }
  };

  const items = data || [];

  return (
    <>
      <PageHeader
        kicker="Операции / Каталог"
        title="Тарифы"
        description="Предложения, которые видит клиент в Mini App и боте."
        actions={
          <button type="button" className="btn primary" onClick={() => setEditing({ ...EMPTY_TARIFF })}>
            Создать тариф
          </button>
        }
      />
      <Card title={`Всего ${count(items.length)}`} description="Нажмите строку для редактирования" padded={false}>
        {error ? (
          <ErrorState detail={error} onRetry={reload} />
        ) : (
          <DataTable
            columns={[
              {
                key: "name",
                title: "Тариф",
                render: (row) => (
                  <span className="cell-stack">
                    <b>{row.name?.ru || row.code}</b>
                    <small>{row.code}</small>
                  </span>
                ),
              },
              { key: "duration", title: "Срок", render: (row) => duration(row.duration_seconds) },
              { key: "traffic", title: "Трафик", render: (row) => bytes(row.traffic_bytes) },
              {
                key: "price",
                title: "Цена",
                align: "right",
                render: (row) => <b className="num">{money(row.amount_minor, row.currency_code)}</b>,
              },
              { key: "position", title: "Позиция", align: "right", render: (row) => row.position, width: "90px" },
              {
                key: "state",
                title: "Статус",
                render: (row) => <Badge tone={row.is_active ? "ok" : "muted"}>{row.is_active ? "активен" : "выключен"}</Badge>,
                width: "120px",
              },
            ]}
            rows={items}
            keyOf={(row) => row.id}
            loading={loading}
            onRowClick={(row) => setEditing({ id: row.id, ...decodeTariff(row) })}
            emptyTitle="Тарифов нет"
            emptyDetail="Создайте первый тариф, чтобы клиенты увидели каталог."
          />
        )}
      </Card>
      {editing && (
        <Modal
          title={editing.id ? "Редактирование тарифа" : "Новый тариф"}
          onClose={() => setEditing(null)}
          footer={
            <>
              <button type="button" className="btn ghost" onClick={() => setEditing(null)}>
                Отмена
              </button>
              <button type="submit" form="tariff-form" className="btn primary" disabled={pending}>
                {pending ? "Сохраняем..." : editing.id ? "Сохранить" : "Создать"}
              </button>
            </>
          }
        >
          <form id="tariff-form" className="form-grid" onSubmit={save}>
            <Field label="Код" hint="Уникальный, латиницей">
              <input
                required
                value={editing.code}
                onChange={(event) => setEditing({ ...editing, code: event.target.value })}
              />
            </Field>
            <Field label="Название (RU)">
              <input required value={editing.name_ru} onChange={(event) => setEditing({ ...editing, name_ru: event.target.value })} />
            </Field>
            <Field label="Название (EN)" hint="Необязательно">
              <input value={editing.name_en} onChange={(event) => setEditing({ ...editing, name_en: event.target.value })} />
            </Field>
            <Field label="Описание (RU)" wide>
              <input
                value={editing.description_ru}
                onChange={(event) => setEditing({ ...editing, description_ru: event.target.value })}
              />
            </Field>
            <Field label="Срок, дней" hint="Пусто — без срока">
              <input
                type="number"
                min="0"
                value={editing.duration_days}
                onChange={(event) => setEditing({ ...editing, duration_days: event.target.value })}
              />
            </Field>
            <Field label="Трафик, ГБ" hint="Пусто — без лимита">
              <input
                type="number"
                min="0"
                value={editing.traffic_gb}
                onChange={(event) => setEditing({ ...editing, traffic_gb: event.target.value })}
              />
            </Field>
            <Field label="Цена, ₽">
              <input
                required
                type="number"
                min="0"
                step="0.01"
                value={editing.amount_rub}
                onChange={(event) => setEditing({ ...editing, amount_rub: event.target.value })}
              />
            </Field>
            <Field label="Позиция" hint="Порядок в каталоге">
              <input
                type="number"
                value={editing.position}
                onChange={(event) => setEditing({ ...editing, position: event.target.value })}
              />
            </Field>
            <div className="form-row">
              <Toggle
                checked={editing.is_active}
                onChange={(value) => setEditing({ ...editing, is_active: value })}
                label="Тариф активен и доступен клиентам"
              />
            </div>
          </form>
        </Modal>
      )}
    </>
  );
}

const EMPTY_PROMO = {
  code: "",
  kind: "discount",
  amount_rub: "",
  discount_percent: "",
  maximum_redemptions: "",
  is_active: true,
};

export function PromosPage({ notify }) {
  const { data, error, loading, reload } = useQuery("/admin/promos");
  const { run, pending } = useMutation();
  const [editing, setEditing] = useState(null);

  const save = async (event) => {
    event.preventDefault();
    const isUpdate = Boolean(editing?.id);
    try {
      await run(`/admin/promos${isUpdate ? `/${editing.id}` : ""}`, {
        method: isUpdate ? "PUT" : "POST",
        body: {
          code: editing.code.trim().toUpperCase(),
          kind: editing.kind,
          amount_minor: editing.kind === "balance" ? Math.round(Number(editing.amount_rub || 0) * 100) : null,
          discount_percent: editing.kind === "discount" ? Number(editing.discount_percent || 0) : null,
          maximum_redemptions: editing.maximum_redemptions === "" ? null : Number(editing.maximum_redemptions),
          is_active: editing.is_active,
          starts_at: null,
          ends_at: null,
        },
      });
      notify({ message: isUpdate ? "Промокод обновлён." : "Промокод создан." });
      setEditing(null);
      reload();
    } catch (cause) {
      notify({ message: cause.message, tone: "error" });
    }
  };

  const items = data || [];

  return (
    <>
      <PageHeader
        kicker="Операции / Маркетинг"
        title="Промокоды"
        description="Начисление на баланс или скидка при оплате."
        actions={
          <button type="button" className="btn primary" onClick={() => setEditing({ ...EMPTY_PROMO })}>
            Создать промокод
          </button>
        }
      />
      <Card title={`Всего ${count(items.length)}`} padded={false}>
        {error ? (
          <ErrorState detail={error} onRetry={reload} />
        ) : (
          <DataTable
            columns={[
              { key: "code", title: "Код", render: (row) => <b className="num">{row.code}</b> },
              { key: "kind", title: "Тип", render: (row) => label(row.kind) },
              {
                key: "value",
                title: "Величина",
                align: "right",
                render: (row) => (row.kind === "balance" ? money(row.amount_minor) : `${row.discount_percent}%`),
              },
              {
                key: "usage",
                title: "Активации",
                align: "right",
                render: (row) => `${count(row.redeemed_count)} / ${row.maximum_redemptions ? count(row.maximum_redemptions) : "∞"}`,
              },
              {
                key: "state",
                title: "Статус",
                render: (row) => <Badge tone={row.is_active ? "ok" : "muted"}>{row.is_active ? "активен" : "выключен"}</Badge>,
                width: "120px",
              },
            ]}
            rows={items}
            keyOf={(row) => row.id}
            loading={loading}
            onRowClick={(row) =>
              setEditing({
                id: row.id,
                code: row.code,
                kind: row.kind,
                amount_rub: row.amount_minor ? String(row.amount_minor / 100) : "",
                discount_percent: row.discount_percent == null ? "" : String(row.discount_percent),
                maximum_redemptions: row.maximum_redemptions == null ? "" : String(row.maximum_redemptions),
                is_active: row.is_active,
              })
            }
            emptyTitle="Промокодов нет"
          />
        )}
      </Card>
      {editing && (
        <Modal
          title={editing.id ? "Редактирование промокода" : "Новый промокод"}
          onClose={() => setEditing(null)}
          footer={
            <>
              <button type="button" className="btn ghost" onClick={() => setEditing(null)}>
                Отмена
              </button>
              <button type="submit" form="promo-form" className="btn primary" disabled={pending}>
                {pending ? "Сохраняем..." : editing.id ? "Сохранить" : "Создать"}
              </button>
            </>
          }
        >
          <form id="promo-form" className="form-grid" onSubmit={save}>
            <Field label="Код">
              <input
                required
                value={editing.code}
                onChange={(event) => setEditing({ ...editing, code: event.target.value.toUpperCase() })}
              />
            </Field>
            <Field label="Тип">
              <select value={editing.kind} onChange={(event) => setEditing({ ...editing, kind: event.target.value })}>
                <option value="discount">Скидка на оплату</option>
                <option value="balance">Начисление на баланс</option>
              </select>
            </Field>
            {editing.kind === "discount" ? (
              <Field label="Скидка, %" hint="От 1 до 100">
                <input
                  required
                  type="number"
                  min="1"
                  max="100"
                  value={editing.discount_percent}
                  onChange={(event) => setEditing({ ...editing, discount_percent: event.target.value })}
                />
              </Field>
            ) : (
              <Field label="Начисление, ₽">
                <input
                  required
                  type="number"
                  min="1"
                  step="0.01"
                  value={editing.amount_rub}
                  onChange={(event) => setEditing({ ...editing, amount_rub: event.target.value })}
                />
              </Field>
            )}
            <Field label="Лимит активаций" hint="Пусто — без лимита">
              <input
                type="number"
                min="1"
                value={editing.maximum_redemptions}
                onChange={(event) => setEditing({ ...editing, maximum_redemptions: event.target.value })}
              />
            </Field>
            <div className="form-row">
              <Toggle
                checked={editing.is_active}
                onChange={(value) => setEditing({ ...editing, is_active: value })}
                label="Промокод активен"
              />
            </div>
          </form>
        </Modal>
      )}
    </>
  );
}

const EMPTY_CHANNEL = { telegram_chat_id: "", title: "", public_url: "", is_active: true };

export function ChannelsPage({ notify }) {
  const { data, error, loading, reload } = useQuery("/admin/required-channels");
  const { run, pending } = useMutation();
  const [editing, setEditing] = useState(null);

  const save = async (event) => {
    event.preventDefault();
    const isUpdate = Boolean(editing?.id);
    try {
      await run(`/admin/required-channels${isUpdate ? `/${editing.id}` : ""}`, {
        method: isUpdate ? "PUT" : "POST",
        body: {
          telegram_chat_id: Number(editing.telegram_chat_id),
          title: editing.title,
          public_url: editing.public_url || null,
          is_active: editing.is_active,
        },
      });
      notify({ message: isUpdate ? "Канал обновлён." : "Канал добавлен." });
      setEditing(null);
      reload();
    } catch (cause) {
      notify({ message: cause.message, tone: "error" });
    }
  };

  const items = data || [];

  return (
    <>
      <PageHeader
        kicker="Доступ / Telegram"
        title="Обязательные каналы"
        description="Без подписки на активные каналы клиент не получит доступ к коммерческим действиям."
        actions={
          <button type="button" className="btn primary" onClick={() => setEditing({ ...EMPTY_CHANNEL })}>
            Добавить канал
          </button>
        }
      />
      <Card title={`Всего ${count(items.length)}`} padded={false}>
        {error ? (
          <ErrorState detail={error} onRetry={reload} />
        ) : (
          <DataTable
            columns={[
              { key: "title", title: "Канал", render: (row) => <b>{row.title}</b> },
              { key: "chat", title: "Chat ID", render: (row) => <span className="num">{row.telegram_chat_id}</span> },
              {
                key: "url",
                title: "Ссылка",
                render: (row) =>
                  row.public_url ? (
                    <a href={row.public_url} target="_blank" rel="noreferrer noopener" onClick={(event) => event.stopPropagation()}>
                      {row.public_url.replace(/^https?:\/\//, "")}
                    </a>
                  ) : (
                    "—"
                  ),
              },
              {
                key: "state",
                title: "Статус",
                render: (row) => <Badge tone={row.is_active ? "ok" : "muted"}>{row.is_active ? "активен" : "выключен"}</Badge>,
                width: "120px",
              },
            ]}
            rows={items}
            keyOf={(row) => row.id}
            loading={loading}
            onRowClick={(row) =>
              setEditing({
                id: row.id,
                telegram_chat_id: String(row.telegram_chat_id),
                title: row.title,
                public_url: row.public_url || "",
                is_active: row.is_active,
              })
            }
            emptyTitle="Каналов нет"
            emptyDetail="Пока список пуст, проверка подписки не выполняется."
          />
        )}
      </Card>
      {editing && (
        <Modal
          title={editing.id ? "Редактирование канала" : "Новый канал"}
          onClose={() => setEditing(null)}
          footer={
            <>
              <button type="button" className="btn ghost" onClick={() => setEditing(null)}>
                Отмена
              </button>
              <button type="submit" form="channel-form" className="btn primary" disabled={pending}>
                {pending ? "Сохраняем..." : editing.id ? "Сохранить" : "Добавить"}
              </button>
            </>
          }
        >
          <form id="channel-form" className="form-grid" onSubmit={save}>
            <Field label="Telegram chat ID" hint="Например -1001234567890">
              <input
                required
                value={editing.telegram_chat_id}
                onChange={(event) => setEditing({ ...editing, telegram_chat_id: event.target.value })}
              />
            </Field>
            <Field label="Название">
              <input required value={editing.title} onChange={(event) => setEditing({ ...editing, title: event.target.value })} />
            </Field>
            <Field label="Публичная ссылка" hint="Необязательно" wide>
              <input
                placeholder="https://t.me/channel"
                value={editing.public_url}
                onChange={(event) => setEditing({ ...editing, public_url: event.target.value })}
              />
            </Field>
            <div className="form-row">
              <Toggle
                checked={editing.is_active}
                onChange={(value) => setEditing({ ...editing, is_active: value })}
                label="Проверять подписку на этот канал"
              />
            </div>
          </form>
        </Modal>
      )}
    </>
  );
}
