import ScreenHead from "../components/ScreenHead";
import { openLink } from "../lib/tg";

export default function Support({ supportUrl, language, t, onBack }) {
  return (
    <>
      <ScreenHead title={t("support")} label="HELP" onBack={onBack} />
      <section className="support-card">
        <p>{t("supportDesc")}</p>
        {supportUrl ? (
          <button className="primary full" onClick={() => openLink(supportUrl)}>
            {language === "ru" ? "Написать в поддержку" : "Contact support"}
          </button>
        ) : (
          <p className="support-empty">
            {language === "ru"
              ? "Контакт поддержки не настроен. Обратитесь к администратору."
              : "Support contact is not configured. Please contact the administrator."}
          </p>
        )}
      </section>
    </>
  );
}
