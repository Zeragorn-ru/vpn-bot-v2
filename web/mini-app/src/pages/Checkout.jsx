import { useState } from "react";
import ScreenHead from "../components/ScreenHead";
import { money } from "../lib/format";
import { haptic } from "../lib/tg";

export default function Checkout({ tariff, providers, me, promo, discount, amount, language, t, title, onBack, onPromo, onRedeem, onWallet, onInvoice }) {
  const [paying, setPaying] = useState(false);
  const available = providers.filter((item) => item.supported_currency_codes.includes(tariff.currency_code));

  const handleWallet = async () => {
    setPaying(true);
    haptic("medium");
    try {
      await onWallet();
    } finally {
      setPaying(false);
    }
  };

  const handleInvoice = async (code) => {
    setPaying(true);
    haptic("medium");
    try {
      await onInvoice(code);
    } finally {
      setPaying(false);
    }
  };

  return (
    <>
      <ScreenHead title={t("checkout")} label="CHECKOUT / 03" onBack={onBack} />
      <section className="checkout-card">
        <span>{title(tariff)}</span>
        <b>{money(tariff.amount_minor, tariff.currency_code, language)}</b>
        <div>
          <small>
            {language === "ru" ? "Итого" : "Total"}
            {discount && ` · -${discount}%`}
          </small>
          <strong>{money(amount, tariff.currency_code, language)}</strong>
        </div>
      </section>

      <form
        className="promo-row"
        onSubmit={(event) => {
          event.preventDefault();
          if (promo.trim()) onRedeem();
        }}
      >
        <input
          value={promo}
          maxLength="64"
          placeholder={t("promo")}
          onChange={(event) => onPromo(event.target.value)}
          disabled={paying}
        />
        <button disabled={paying}>{t("apply")}</button>
      </form>

      {tariff.currency_code === me.currency_code && (
        <button className="primary full" onClick={handleWallet} disabled={paying}>
          {paying ? "..." : t("pay")}
        </button>
      )}

      <section className="provider-list">
        <p>{t("payment")}</p>
        {discount ? (
          <small>{language === "ru" ? "Скидка применяется при оплате с кошелька." : "Discount applies to wallet purchases."}</small>
        ) : (
          available.map((provider) => (
            <button key={provider.code} onClick={() => handleInvoice(provider.code)} disabled={paying}>
              <span>{provider.code.replaceAll("_", " ")}</span>
              {paying ? "..." : "→"}
            </button>
          ))
        )}
      </section>
    </>
  );
}
