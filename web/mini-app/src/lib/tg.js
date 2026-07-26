const tg = window.Telegram?.WebApp;

export const telegram = tg;

export function initTelegram() {
  if (!tg) return;
  tg.ready();
  tg.expand();
  tg.setHeaderColor("#080d1f");
  tg.setBackgroundColor("#080d1f");
  if (tg.BackButton) {
    tg.BackButton.hide();
  }
}

export function showBackButton(onClick) {
  if (!tg?.BackButton) return;
  tg.BackButton.show();
  tg.BackButton.offClick();
  tg.BackButton.onClick(onClick);
}

export function hideBackButton() {
  if (!tg?.BackButton) return;
  tg.BackButton.hide();
}

export function setMainButton(text, onClick, options = {}) {
  if (!tg?.MainButton) return;
  tg.MainButton.setText(text);
  tg.MainButton.offClick();
  tg.MainButton.onClick(onClick);
  if (options.color) tg.MainButton.setParams({ color: options.color });
  if (options.textColor) tg.MainButton.setParams({ text_color: options.textColor });
  tg.MainButton.show();
}

export function hideMainButton() {
  if (!tg?.MainButton) return;
  tg.MainButton.hide();
}

export function haptic(type = "light") {
  if (!tg?.HapticFeedback) return;
  switch (type) {
    case "light": tg.HapticFeedback.impactOccurred("light"); break;
    case "medium": tg.HapticFeedback.impactOccurred("medium"); break;
    case "heavy": tg.HapticFeedback.impactOccurred("heavy"); break;
    case "success": tg.HapticFeedback.notificationOccurred("success"); break;
    case "error": tg.HapticFeedback.notificationOccurred("error"); break;
    default: tg.HapticFeedback.impactOccurred("light");
  }
}

export function openLink(url) {
  if (tg?.openLink) tg.openLink(url);
  else window.open(url, "_blank", "noopener,noreferrer");
}

export function openInvoice(url, callback) {
  if (tg?.openInvoice) tg.openInvoice(url, callback);
  else openLink(url);
}

export function close() {
  if (tg?.close) tg.close();
}
