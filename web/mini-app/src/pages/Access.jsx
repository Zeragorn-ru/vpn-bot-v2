import ScreenHead from "../components/ScreenHead";
import { haptic } from "../lib/tg";

export default function Access({ url, t, onBack, onCopy }) {
  return (
    <>
      <ScreenHead title={t("access")} label="ROUTE / 04" onBack={onBack} />
      <section className="access-card">
        <span>CONNECTION LINK</span>
        <code>{url}</code>
        <button
          className="primary full"
          onClick={() => {
            haptic("success");
            onCopy();
          }}
        >
          {t("copy")}
        </button>
      </section>
      <section className="guide-card">
        <p>01 / 02 / 03</p>
        <h2>{t("instructions")}</h2>
        <ol>
          <li>Install Happ, v2rayNG, Karing, or Clash.</li>
          <li>Copy and import this protected link.</li>
          <li>Connect. Your client receives a compatible format.</li>
        </ol>
      </section>
    </>
  );
}
