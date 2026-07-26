import ScreenHead from "../components/ScreenHead";
import { money, formatDate, formatTrafficGB, formatDuration } from "../lib/format";
import { haptic } from "../lib/tg";

export default function Subscription({ subscription, language, t, onBack, onAccess }) {
  if (!subscription) {
    return (
      <>
        <ScreenHead title={t("subscription")} label="SUB" onBack={onBack} />
        <section className="sub-card empty">
          <p>{t("noSubscription")}</p>
        </section>
      </>
    );
  }

  const active = subscription.status === "active";
  const pending = subscription.status === "provisioning_pending";
  const statusText = active ? t("statusActive") : pending ? t("statusPending") : t("statusExpired");
  const trafficPercent = subscription.traffic_bytes
    ? Math.min(100, Math.round(((subscription.traffic_used_bytes || 0) / subscription.traffic_bytes) * 100))
    : 0;

  return (
    <>
      <ScreenHead title={t("subscription")} label="SUB" onBack={onBack} />

      <section className="sub-card">
        <div className="sub-header">
          <span className={`badge badge-${subscription.status}`}>{statusText}</span>
          {subscription.is_trial && <span className="badge badge-trial">{t("trialBadge")}</span>}
        </div>

        <div className="sub-field">
          <span>{t("tariff")}</span>
          <strong>{subscription.tariff_name || subscription.tariff_code || "-"}</strong>
        </div>

        <div className="sub-field">
          <span>{t("startDate")}</span>
          <span>{formatDate(subscription.created_at, language)}</span>
        </div>

        <div className="sub-field">
          <span>{t("expiresAt")}</span>
          <strong>{formatDate(subscription.expires_at, language)}</strong>
        </div>

        {subscription.traffic_bytes > 0 && (
          <div className="sub-traffic">
            <div className="sub-traffic-header">
              <span>{t("trafficUsed")}</span>
              <span>{formatTrafficGB(subscription.traffic_used_bytes || 0)} / {formatTrafficGB(subscription.traffic_bytes)}</span>
            </div>
            <div className="traffic-bar">
              <div className="traffic-fill" style={{ width: `${trafficPercent}%` }} />
            </div>
          </div>
        )}
      </section>

      {subscription.access_available && (
        <button
          className="primary full"
          onClick={() => {
            haptic("medium");
            onAccess();
          }}
        >
          {t("connectVpn")}
        </button>
      )}
    </>
  );
}
