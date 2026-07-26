import { StrictMode, useEffect, useMemo, useState, useCallback } from "react";
import { createRoot } from "react-dom/client";
import { createClient, authenticateTelegram } from "./lib/api";
import { useI18n } from "./lib/i18n";
import { money } from "./lib/format";
import {
  initTelegram,
  showBackButton,
  hideBackButton,
  haptic,
} from "./lib/tg";
import State from "./components/State";
import Notice from "./components/Notice";
import Home from "./pages/Home";
import Catalog from "./pages/Catalog";
import Checkout from "./pages/Checkout";
import Access from "./pages/Access";
import Support from "./pages/Support";
import History from "./pages/History";
import Subscription from "./pages/Subscription";
import Invoice from "./pages/Invoice";
import "./styles.css";

const preview = new URLSearchParams(location.search).get("preview") === "1";

function previewData() {
  return {
    me: { language_code: "ru", balance_minor: 14900, currency_code: "RUB" },
    subscription: {
      id: "preview-subscription",
      status: "active",
      expires_at: "2026-08-24T12:00:00Z",
      traffic_bytes: 100000000000,
      traffic_used_bytes: 35000000000,
      access_available: true,
      tariff_name: "Месяц",
      tariff_code: "month",
      is_trial: false,
      created_at: "2026-07-24T12:00:00Z",
    },
    tariffs: [
      {
        id: "month",
        code: "month",
        name: { ru: "Месяц", en: "One month" },
        amount_minor: 29900,
        currency_code: "RUB",
        duration_seconds: 2592000,
        traffic_bytes: 100000000000,
      },
      {
        id: "quarter",
        code: "quarter",
        name: { ru: "Три месяца", en: "Three months" },
        amount_minor: 79900,
        currency_code: "RUB",
        duration_seconds: 7776000,
        traffic_bytes: 300000000000,
      },
    ],
    providers: [
      { code: "crypto_pay", supported_currency_codes: ["RUB"] },
      { code: "anore", supported_currency_codes: ["RUB"] },
    ],
  };
}

export default function App() {
  const [token, setToken] = useState(null);
  const [data, setData] = useState(null);
  const [screen, setScreen] = useState("home");
  const [selected, setSelected] = useState(null);
  const [accessUrl, setAccessUrl] = useState(null);
  const [promo, setPromo] = useState("");
  const [discount, setDiscount] = useState(null);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [invoiceId, setInvoiceId] = useState(null);

  const language = data?.me?.language_code || "ru";
  const t = useI18n(language);

  const api = useMemo(
    () =>
      createClient(() => {
        if (preview) return "preview";
        return token;
      }),
    [token]
  );

  const refresh = useCallback(
    async (sessionToken = token) => {
      if (preview) return;
      const [me, subscription, tariffs, providers] = await Promise.all([
        api.get("/me"),
        api.get("/subscriptions/current"),
        api.get("/tariffs"),
        api.get("/payment-providers"),
      ]);
      setData({ me, subscription, tariffs, providers });
    },
    [api, token]
  );

  const bootstrap = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      if (preview) {
        setData(previewData());
        return;
      }
      initTelegram();
      const saved = JSON.parse(
        sessionStorage.getItem("vpn-mini-app-session") || "null"
      );
      let session =
        saved?.token && saved.expiresAt > Date.now() ? saved.token : null;
      if (!session) {
        if (!window.Telegram?.WebApp?.initData)
          throw new Error(t("unavailable"));
        const auth = await authenticateTelegram(
          window.Telegram.WebApp.initData
        );
        session = auth.access_token;
        sessionStorage.setItem(
          "vpn-mini-app-session",
          JSON.stringify({
            token: session,
            expiresAt: new Date(auth.expires_at).getTime(),
          })
        );
      }
      setToken(session);
      const client = createClient(() => session);
      const [me, subscription, tariffs, providers] = await Promise.all([
        client.get("/me"),
        client.get("/subscriptions/current"),
        client.get("/tariffs"),
        client.get("/payment-providers"),
      ]);
      setData({ me, subscription, tariffs, providers });
    } catch (cause) {
      sessionStorage.removeItem("vpn-mini-app-session");
      setError(cause.message);
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    bootstrap();
  }, []);

  useEffect(() => {
    if (!loading && data) {
      const deepScreen = new URLSearchParams(location.search).get("screen");
      if (deepScreen && ["catalog", "access", "support", "history", "subscription"].includes(deepScreen)) {
        setScreen(deepScreen);
        window.history.replaceState({}, "", location.pathname);
      }
    }
  }, [loading, data]);

  const navigate = useCallback(
    (screenName) => {
      haptic("light");
      setScreen(screenName);
    },
    []
  );

  useEffect(() => {
    if (screen === "home") {
      hideBackButton();
    } else {
      showBackButton(() => {
        haptic("light");
        if (screen === "invoice") {
          setScreen("home");
        } else {
          setScreen("home");
        }
      });
    }
  }, [screen]);

  const selectedTariff = useMemo(
    () => data?.tariffs.find((item) => item.id === selected) || null,
    [data, selected]
  );

  const mutate = useCallback(
    async (action, success) => {
      try {
        await action();
        await success();
      } catch (cause) {
        setNotice(cause.message);
      }
    },
    []
  );

  const openAccess = useCallback(async () => {
    if (preview) {
      setAccessUrl("http://31.77.203.80:18081/sub/demo?token=preview-only");
      setScreen("access");
      return;
    }
    await mutate(
      async () => {
        const response = await api.get(
          `/subscriptions/${data.subscription.id}/access`
        );
        setAccessUrl(response.access_url);
        setScreen("access");
      },
      async () => {}
    );
  }, [api, data, mutate]);

  const changeLanguage = useCallback(async () => {
    if (preview) {
      setData((current) => ({
        ...current,
        me: {
          ...current.me,
          language_code: language === "ru" ? "en" : "ru",
        },
      }));
      return;
    }
    await mutate(
      async () => {
        const me = await api.put("/me", {
          language_code: language === "ru" ? "en" : "ru",
        });
        setData((current) => ({ ...current, me }));
      },
      async () => {}
    );
  }, [api, language, mutate]);

  const purchase = useCallback(async () => {
    if (preview) {
      setNotice(t("processing"));
      setScreen("home");
      return;
    }
    await mutate(
      () =>
        api.post("/purchases", {
          tariff_id: selected,
          promo_code: promo || null,
        }),
      async () => {
        setNotice(t("processing"));
        setPromo("");
        setDiscount(null);
        setScreen("home");
        await refresh();
      }
    );
  }, [api, selected, promo, mutate, refresh, t]);

  const trial = useCallback(async () => {
    if (preview) {
      setNotice(t("processing"));
      return;
    }
    await mutate(
      () => api.post("/trials"),
      async () => {
        setNotice(t("processing"));
        await refresh();
      }
    );
  }, [api, mutate, refresh, t]);

  const redeem = useCallback(async () => {
    await mutate(
      async () => {
        const result = await api.get(
          `/promos/${encodeURIComponent(promo.trim())}/preview`
        );
        if (result.kind === "discount") {
          setPromo(result.code);
          setDiscount(result.discount_percent);
          setNotice(`-${result.discount_percent}%`);
          return;
        }
        const creditResult = await api.post("/promos/redeem", {
          code: promo.trim(),
        });
        setPromo("");
        setNotice(
          `${creditResult.code}: +${money(
            creditResult.credited_amount_minor,
            creditResult.currency_code,
            language
          )}`
        );
        await refresh();
      },
      async () => {}
    );
  }, [api, promo, language, mutate, refresh]);

  const invoice = useCallback(
    async (provider) => {
      if (preview) {
        setNotice(t("paymentCreated"));
        setScreen("home");
        return;
      }
      await mutate(
        async () => {
          const result = await api.post("/invoices", {
            provider,
            currency_code: selectedTariff.currency_code,
            tariff_id: selected,
          });
          setInvoiceId(result.id);
          setScreen("invoice");
        },
        async () => {}
      );
    },
    [api, selected, selectedTariff, mutate, t]
  );

  if (loading) return <State title={t("loading")} />;
  if (error)
    return (
      <State title={error} action={bootstrap} actionLabel={t("retry")} error />
    );

  const { me, subscription, tariffs, providers } = data;
  const tariffTitle = (item) =>
    item.name[language] || item.name.ru || item.name.en || item.code;
  const amount =
    selectedTariff && discount
      ? Math.floor((selectedTariff.amount_minor * (100 - discount)) / 100)
      : selectedTariff?.amount_minor;

  return (
    <main className="mini-shell">
      <header className="mini-topbar">
        <button className="wordmark" onClick={() => navigate("home")}>
          <i>V</i>VECTOR
        </button>
        <button className="lang" onClick={changeLanguage}>
          {language === "ru" ? "EN" : "RU"}
        </button>
      </header>

      <Notice message={notice} onClose={() => setNotice("")} />

      {screen === "home" && (
        <Home
          subscription={subscription}
          me={me}
          language={language}
          t={t}
          onAccess={openAccess}
          onCatalog={() => navigate("catalog")}
          onTrial={trial}
          onHistory={() => navigate("history")}
          onSupport={() => navigate("support")}
          onSubscriptionDetail={() => navigate("subscription")}
        />
      )}
      {screen === "catalog" && (
        <Catalog
          tariffs={tariffs}
          selected={selected}
          language={language}
          t={t}
          title={tariffTitle}
          onBack={() => navigate("home")}
          onSelect={setSelected}
          onContinue={() => navigate("checkout")}
        />
      )}
      {screen === "checkout" && (
        <Checkout
          tariff={selectedTariff}
          providers={providers}
          me={me}
          promo={promo}
          discount={discount}
          amount={amount}
          language={language}
          t={t}
          title={tariffTitle}
          onBack={() => navigate("catalog")}
          onPromo={setPromo}
          onRedeem={redeem}
          onWallet={purchase}
          onInvoice={invoice}
        />
      )}
      {screen === "access" && (
        <Access
          url={accessUrl}
          language={language}
          t={t}
          onBack={() => navigate("home")}
          onCopy={() => {
            haptic("success");
            navigator.clipboard
              .writeText(accessUrl)
              .then(() =>
                setNotice(
                  language === "ru" ? "Ссылка скопирована" : "Link copied"
                )
              );
          }}
        />
      )}
      {screen === "support" && (
        <Support
          supportUrl={data.publicSettings?.support_url}
          language={language}
          t={t}
          onBack={() => navigate("home")}
        />
      )}
      {screen === "history" && (
        <History
          api={api}
          language={language}
          t={t}
          onBack={() => navigate("home")}
        />
      )}
      {screen === "subscription" && (
        <Subscription
          subscription={subscription}
          language={language}
          t={t}
          onBack={() => navigate("home")}
          onAccess={openAccess}
        />
      )}
      {screen === "invoice" && (
        <Invoice
          api={api}
          invoiceId={invoiceId}
          language={language}
          t={t}
          onBack={() => navigate("home")}
          onPaid={async () => {
            await refresh();
            navigate("home");
            setNotice(t("processing"));
          }}
        />
      )}
    </main>
  );
}

createRoot(document.getElementById("app")).render(
  <StrictMode>
    <App />
  </StrictMode>
);
