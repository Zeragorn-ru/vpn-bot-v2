export function money(value, currency, language) {
  const divisor = currency === "XTR" ? 1 : 100;
  const locale = language === "ru" ? "ru-RU" : "en-US";
  return new Intl.NumberFormat(locale, {
    style: "currency",
    currency,
    maximumFractionDigits: currency === "XTR" ? 0 : 2,
  }).format(value / divisor);
}

export function formatDate(value, language) {
  if (!value) return "-";
  const locale = language === "ru" ? "ru-RU" : "en-US";
  return new Intl.DateTimeFormat(locale, {
    day: "numeric",
    month: "short",
    year: "numeric",
  }).format(new Date(value));
}

export function formatDateTime(value, language) {
  if (!value) return "-";
  const locale = language === "ru" ? "ru-RU" : "en-US";
  return new Intl.DateTimeFormat(locale, {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}

export function formatBytes(bytes, language) {
  if (bytes == null) return "-";
  const units = language === "ru" ? ["Б", "КБ", "МБ", "ГБ", "ТБ"] : ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let value = bytes;
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024;
    i++;
  }
  return `${value.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

export function formatTrafficGB(bytes) {
  if (bytes == null) return "-";
  return `${Math.round(bytes / 1e9)} GB`;
}

export function formatDuration(seconds, language) {
  if (!seconds) return "-";
  const days = Math.round(seconds / 86400);
  if (language === "ru") {
    if (days === 1) return "1 день";
    if (days < 5) return `${days} дня`;
    return `${days} дней`;
  }
  return days === 1 ? "1 day" : `${days} days`;
}

export function timeAgo(value, language) {
  if (!value) return "";
  const diff = Date.now() - new Date(value).getTime();
  const minutes = Math.floor(diff / 60000);
  const hours = Math.floor(diff / 3600000);
  const days = Math.floor(diff / 86400000);
  if (language === "ru") {
    if (minutes < 1) return "только что";
    if (minutes < 60) return `${minutes} мин. назад`;
    if (hours < 24) return `${hours} ч. назад`;
    return `${days} дн. назад`;
  }
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  if (hours < 24) return `${hours}h ago`;
  return `${days}d ago`;
}
