import { useState, useEffect, useRef } from "react";
import ScreenHead from "../components/ScreenHead";
import { money } from "../lib/format";
import { haptic } from "../lib/tg";

export default function Invoice({ api, invoiceId, language, t, onBack, onPaid }) {
  const [invoice, setInvoice] = useState(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const pollRef = useRef(null);

  useEffect(() => {
    if (!invoiceId) return;
    let cancelled = false;

    const poll = async () => {
      try {
        const data = await api.get(`/invoices/${invoiceId}`);
        if (cancelled) return;
        setInvoice(data);
        setLoading(false);

        if (data.status === "paid") {
          haptic("success");
          onPaid();
          return;
        }
        if (data.status === "expired" || data.status === "cancelled") {
          haptic("error");
          return;
        }
        pollRef.current = setTimeout(poll, 3000);
      } catch (err) {
        if (!cancelled) {
          setError(err.message);
          setLoading(false);
        }
      }
    };

    poll();
    return () => {
      cancelled = true;
      if (pollRef.current) clearTimeout(pollRef.current);
    };
  }, [invoiceId]);

  const statusText = {
    pending: t("invoicePending"),
    paid: t("invoicePaid"),
    expired: t("invoiceExpired"),
    cancelled: t("invoiceCancelled"),
  };

  return (
    <>
      <ScreenHead title={t("checkout")} label="INVOICE" onBack={onBack} />
      {loading && (
        <section className="invoice-status">
          <div className="invoice-spinner" />
          <p>{t("processing")}</p>
        </section>
      )}
      {error && <p className="state-text error">{error}</p>}
      {invoice && (
        <section className="invoice-card">
          <div className="invoice-amount">{money(invoice.amount_minor, invoice.currency_code, language)}</div>
          <span className={`badge badge-${invoice.status}`}>{statusText[invoice.status] || invoice.status}</span>
          {invoice.status === "pending" && (
            <p className="invoice-hint">
              {language === "ru"
                ? "Ожидаем подтверждение оплаты. Это может занять несколько минут."
                : "Waiting for payment confirmation. This may take a few minutes."}
            </p>
          )}
          {invoice.status === "paid" && (
            <p className="invoice-hint success">
              {language === "ru" ? "Оплата получена!" : "Payment received!"}
            </p>
          )}
          {(invoice.status === "expired" || invoice.status === "cancelled") && (
            <button
              className="primary full"
              onClick={() => {
                haptic("light");
                onBack();
              }}
            >
              {t("back")}
            </button>
          )}
        </section>
      )}
    </>
  );
}
