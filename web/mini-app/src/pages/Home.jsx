import { money, formatDate, formatTrafficGB } from "../lib/format";
import { haptic } from "../lib/tg";

export default function Home({ subscription, me, language, t, onAccess, onCatalog, onTrial, onHistory, onSupport, onSubscriptionDetail }) {
  const active = subscription?.status === "active";
  const pending = subscription?.status === "provisioning_pending";
  const status = active ? t("active") : pending ? t("pending") : t("inactive");

  return (
    <>
      <section className="route-hero">
        <div className="route-grid" />
        <p>SECURE ROUTE / 01</p>
        <h1>{t("title")}</h1>
        <div className="route-status">
          <i className={active ? "on" : ""} />
          {status}
        </div>
        {active && <strong>{formatDate(subscription.expires_at, language)}</strong>}
        <div className="route-actions">
          <button
            className="primary"
            disabled={!subscription?.access_available}
            onClick={() => {
              haptic("medium");
              onAccess();
            }}
          >
            {t("open")}
          </button>
          <button
            className="secondary"
            onClick={() => {
              haptic("light");
              onCatalog();
            }}
          >
            {t("renew")}
          </button>
        </div>
      </section>

      {!subscription && (
        <button
          className="trial-cta"
          onClick={() => {
            haptic("medium");
            onTrial();
          }}
        >
          {t("trial")}
          <span>10 GB →</span>
        </button>
      )}

      {subscription && (
        <section
          className="info-card clickable"
          onClick={() => {
            haptic("light");
            onSubscriptionDetail();
          }}
        >
          <div className="info-row">
            <span>{t("subscription")}</span>
            <span className="badge">{active ? t("statusActive") : pending ? t("statusPending") : t("statusExpired")}</span>
          </div>
          {active && (
            <div className="info-row">
              <span>{t("trafficUsed")}</span>
              <span>{formatTrafficGB(subscription.traffic_used_bytes)} / {formatTrafficGB(subscription.traffic_bytes)}</span>
            </div>
          )}
          <div className="info-row">
            <span>{t("expiresAt")}</span>
            <span>{formatDate(subscription.expires_at, language)}</span>
          </div>
          <span className="info-arrow">→</span>
        </section>
      )}

      <section className="balance-card">
        <div>
          <span>{t("wallet")}</span>
          <b>{money(me.balance_minor, me.currency_code, language)}</b>
        </div>
        <button
          onClick={() => {
            haptic("light");
            onCatalog();
          }}
        >
          {t("topUp")}
        </button>
      </section>

      <section className="mini-section">
        <p>PLAN / 02</p>
        <button onClick={onCatalog}>{t("plans")}</button>
      </section>

      <nav className="mini-nav">
        <button onClick={onHistory}>{t("history")}</button>
        <button onClick={onSupport}>{t("support")}</button>
      </nav>
    </>
  );
}
