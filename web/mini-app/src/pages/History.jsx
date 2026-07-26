import { useState, useEffect } from "react";
import ScreenHead from "../components/ScreenHead";
import { money, formatDateTime } from "../lib/format";

export default function History({ api, language, t, onBack }) {
  const [invoices, setInvoices] = useState([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const data = await api.get("/invoices?limit=50");
        if (!cancelled) setInvoices(data || []);
      } catch (err) {
        if (!cancelled) setError(err.message);
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, []);

  const statusLabel = (status) => {
    const map = {
      pending: t("invoicePending"),
      paid: t("invoicePaid"),
      expired: t("invoiceExpired"),
      cancelled: t("invoiceCancelled"),
    };
    return map[status] || status;
  };

  return (
    <>
      <ScreenHead title={t("history")} label="HISTORY" onBack={onBack} />
      {loading && <p className="state-text">{t("loading")}</p>}
      {error && <p className="state-text error">{error}</p>}
      {!loading && !error && invoices.length === 0 && (
        <p className="state-text">{t("noHistory")}</p>
      )}
      <section className="history-list">
        {invoices.map((inv) => (
          <div className="history-item" key={inv.id}>
            <div className="history-row">
              <span className={`badge badge-${inv.status}`}>{statusLabel(inv.status)}</span>
              <strong>{money(inv.amount_minor, inv.currency_code, language)}</strong>
            </div>
            <div className="history-row">
              <small>{inv.purpose === "wallet_top_up" ? t("topUp") : (inv.tariff_name || t("subscription"))}</small>
              <small>{formatDateTime(inv.paid_at || inv.created_at, language)}</small>
            </div>
          </div>
        ))}
      </section>
    </>
  );
}
