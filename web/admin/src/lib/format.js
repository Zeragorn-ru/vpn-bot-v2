const MINOR_UNITS = { XTR: 1, RUB: 100, USD: 100, EUR: 100 };

const minorFactor = (currency) => MINOR_UNITS[currency] ?? 100;

export const money = (value, currency = "RUB") => {
  const factor = minorFactor(currency);
  try {
    return new Intl.NumberFormat("ru-RU", {
      style: "currency",
      currency,
      maximumFractionDigits: factor === 1 ? 0 : 2,
    }).format((Number(value) || 0) / factor);
  } catch {
    return `${((Number(value) || 0) / factor).toLocaleString("ru-RU")} ${currency}`;
  }
};

export const compactMoney = (value, currency = "RUB") => {
  const factor = minorFactor(currency);
  const amount = (Number(value) || 0) / factor;
  if (Math.abs(amount) >= 1000) {
    return `${new Intl.NumberFormat("ru-RU", { notation: "compact", maximumFractionDigits: 1 }).format(amount)} ₽`;
  }
  return money(value, currency);
};

export const count = (value) => new Intl.NumberFormat("ru-RU").format(Number(value) || 0);

export const percent = (value, fractionDigits = 1) =>
  value == null
    ? "—"
    : `${Number(value) > 0 ? "+" : ""}${Number(value).toFixed(fractionDigits).replace(/\.0$/, "")}%`;

export const date = (value) =>
  value
    ? new Intl.DateTimeFormat("ru-RU", { day: "2-digit", month: "short", year: "numeric" }).format(new Date(value))
    : "—";

export const dateTime = (value) =>
  value
    ? new Intl.DateTimeFormat("ru-RU", {
        day: "2-digit",
        month: "short",
        hour: "2-digit",
        minute: "2-digit",
      }).format(new Date(value))
    : "—";

export const dayLabel = (value) => {
  if (!value) return "";
  const parsed = new Date(`${value}T00:00:00`);
  return new Intl.DateTimeFormat("ru-RU", { day: "2-digit", month: "short" }).format(parsed);
};

export const timeAgo = (value) => {
  if (!value) return "";
  const diff = Date.now() - new Date(value).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "только что";
  if (mins < 60) return `${mins} мин`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs} ч`;
  const days = Math.floor(hrs / 24);
  if (days < 30) return `${days} дн`;
  return date(value);
};

export const bytes = (value) => {
  if (value == null) return "без лимита";
  const gb = Number(value) / 1024 ** 3;
  if (gb >= 1024) return `${(gb / 1024).toFixed(1)} ТБ`;
  if (gb >= 1) return `${Math.round(gb)} ГБ`;
  return `${Math.max(1, Math.round(Number(value) / 1024 ** 2))} МБ`;
};

export const duration = (seconds) => {
  if (!seconds) return "без срока";
  const days = Math.round(Number(seconds) / 86400);
  if (days % 365 === 0) return `${days / 365} г.`;
  if (days % 30 === 0) return `${days / 30} мес.`;
  return `${days} дн.`;
};

export const providerNames = {
  crypto_pay: "Crypto Pay",
  anore: "Anore",
  telegram_stars: "Telegram Stars",
};

const LABELS = {
  paid: "оплачен",
  pending: "ожидает",
  expired: "истёк",
  cancelled: "отменён",
  active: "активна",
  provisioning_pending: "в обработке",
  suspended: "приостановлена",
  failed: "ошибка",
  wallet_top_up: "пополнение",
  direct_purchase: "покупка",
  balance: "баланс",
  discount: "скидка",
};

export const label = (value) => LABELS[value] || String(value ?? "").replaceAll("_", " ");

export const statusTone = (value) =>
  ({
    paid: "ok",
    active: "ok",
    pending: "warn",
    provisioning_pending: "warn",
    suspended: "warn",
    expired: "muted",
    cancelled: "muted",
    failed: "error",
  })[value] || "muted";

export const providerLabel = (code) => providerNames[code] || label(code);

export const userLabel = (item) =>
  item?.username ? `@${item.username}` : item?.first_name || `Telegram ${item?.telegram_user_id ?? "—"}`;
