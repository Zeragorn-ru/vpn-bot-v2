import ScreenHead from "../components/ScreenHead";
import { money, formatDuration, formatTrafficGB } from "../lib/format";
import { haptic } from "../lib/tg";

export default function Catalog({ tariffs, selected, language, t, title, onBack, onSelect, onContinue }) {
  return (
    <>
      <ScreenHead title={t("plans")} label="PLAN / 02" onBack={onBack} />
      <section className="plan-stack">
        {tariffs.map((tariff) => (
          <button
            className={`plan-card ${selected === tariff.id ? "selected" : ""}`}
            key={tariff.id}
            onClick={() => {
              haptic("light");
              onSelect(tariff.id);
            }}
          >
            <span>{title(tariff)}</span>
            <b>{money(tariff.amount_minor, tariff.currency_code, language)}</b>
            <small>
              {tariff.duration_seconds ? formatDuration(tariff.duration_seconds, language) : ""}
              {tariff.traffic_bytes ? ` · ${formatTrafficGB(tariff.traffic_bytes)}` : ""}
            </small>
          </button>
        ))}
      </section>
      <button
        className="primary sticky"
        disabled={!selected}
        onClick={() => {
          haptic("medium");
          onContinue();
        }}
      >
        {t("choose")}
      </button>
    </>
  );
}
