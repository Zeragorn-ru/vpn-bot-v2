import { useState } from "react";
import { useQueries } from "../lib/api.jsx";
import {
  compactMoney,
  count,
  dayLabel,
  label,
  money,
  providerLabel,
  timeAgo,
} from "../lib/format.js";
import { AreaChart, BarChart, DonutChart, RankedBars } from "../components/charts.jsx";
import {
  Badge,
  Card,
  ErrorState,
  EmptyState,
  Loader,
  MetricCard,
  PageHeader,
  SegmentedControl,
  Skeleton,
} from "../components/ui.jsx";

const PERIODS = [
  [7, "7 дней"],
  [14, "14 дней"],
  [30, "30 дней"],
  [90, "90 дней"],
];

const series = (points) => (points || []).map((point) => Number(point.value));
const labels = (points) => (points || []).map((point) => dayLabel(point.day));

export default function Overview() {
  const [days, setDays] = useState(() => Number(localStorage.getItem("vpn-analytics-days")) || 14);
  const { data, error, loading, reload } = useQueries([
    ["dashboard", "/admin/dashboard"],
    ["analytics", "/admin/analytics", { params: { days } }],
    ["audit", "/admin/audit", { params: { limit: 6, offset: 0 } }],
  ]);

  const changePeriod = (value) => {
    setDays(Number(value));
    localStorage.setItem("vpn-analytics-days", String(value));
  };

  if (loading && !data.dashboard) return <Loader title="Собираем операционные показатели" />;
  if (error && !data.dashboard) return <ErrorState detail={error} onRetry={reload} />;

  const dashboard = data.dashboard || {};
  const analytics = data.analytics || {};
  const totals = analytics.totals || {};
  const auditItems = data.audit?.items || [];
  const attention = (dashboard.provisioning_pending_subscriptions ?? 0) + (dashboard.pending_invoices ?? 0);
  const paidInvoices = totals.paid_invoices?.current ?? 0;
  const registrations = totals.registrations?.current ?? 0;
  const conversion = registrations > 0 ? Math.round((paidInvoices / registrations) * 100) : null;

  return (
    <>
      <PageHeader
        kicker="Главное / Обзор"
        title="Операционный центр"
        description="Выручка, привлечение и очереди выдачи за выбранный период."
        actions={
          <SegmentedControl value={days} options={PERIODS} onChange={changePeriod} ariaLabel="Период аналитики" />
        }
      />

      <div className="metric-grid">
        <MetricCard
          label="Выручка"
          value={money(totals.revenue_rub_minor?.current ?? 0)}
          caption={`пред. ${compactMoney(totals.revenue_rub_minor?.previous ?? 0)}`}
          delta={totals.revenue_rub_minor?.change_percent}
          trend={series(analytics.revenue_rub_minor)}
          color="var(--chart-1)"
        />
        <MetricCard
          label="Оплаченные счета"
          value={count(paidInvoices)}
          caption={`пред. ${count(totals.paid_invoices?.previous ?? 0)}`}
          delta={totals.paid_invoices?.change_percent}
          trend={series(analytics.paid_invoices)}
          color="var(--chart-2)"
        />
        <MetricCard
          label="Регистрации"
          value={count(registrations)}
          caption={`пред. ${count(totals.registrations?.previous ?? 0)}`}
          delta={totals.registrations?.change_percent}
          trend={series(analytics.registrations)}
          color="var(--chart-3)"
        />
        <MetricCard
          label="Новые подписки"
          value={count(totals.subscriptions?.current ?? 0)}
          caption={`пробных ${count(totals.trials?.current ?? 0)}`}
          delta={totals.subscriptions?.change_percent}
          trend={series(analytics.new_subscriptions)}
          color="var(--chart-4)"
        />
      </div>

      <div className="grid two">
        <Card
          title="Выручка"
          description={`Оплаченные счёта в рублях, ${days} дн.`}
          action={<b className="card-total">{money(totals.revenue_rub_minor?.current ?? 0)}</b>}
        >
          {loading && !analytics.revenue_rub_minor ? (
            <Skeleton rows={3} />
          ) : (
            <AreaChart
              series={[{ name: "Выручка", values: series(analytics.revenue_rub_minor), color: "var(--chart-1)" }]}
              labels={labels(analytics.revenue_rub_minor)}
              formatValue={(value) => money(value)}
              formatTick={(value) => compactMoney(value)}
            />
          )}
        </Card>
        <Card
          title="Привлечение"
          description="Регистрации и выданные подписки"
          action={<b className="card-total">{count(registrations)}</b>}
        >
          {loading && !analytics.registrations ? (
            <Skeleton rows={3} />
          ) : (
            <AreaChart
              series={[
                { name: "Регистрации", values: series(analytics.registrations), color: "var(--chart-3)" },
                { name: "Подписки", values: series(analytics.new_subscriptions), color: "var(--chart-2)" },
              ]}
              labels={labels(analytics.registrations)}
              formatValue={count}
              formatTick={count}
            />
          )}
        </Card>
      </div>

      <div className="grid two">
        <Card title="Оплаты по дням" description={`Количество успешных платежей, ${days} дн.`}>
          <BarChart
            values={series(analytics.paid_invoices)}
            labels={labels(analytics.paid_invoices)}
            color="var(--chart-2)"
            formatValue={count}
            formatTick={count}
          />
        </Card>
        <Card title="Выручка по провайдерам" description="Доля платёжных адаптеров за период">
          <DonutChart
            entries={(analytics.revenue_by_provider || []).map((entry) => ({
              key: entry.key,
              label: providerLabel(entry.key),
              value: entry.value,
            }))}
            formatValue={(value) => compactMoney(value)}
            emptyLabel="Оплат за период не было"
          />
        </Card>
      </div>

      <div className="grid three">
        <Card title="Требует внимания" description="Незакрытые операционные очереди">
          <ul className="stat-rows">
            <li>
              <span>Ожидают оплаты</span>
              <b>{count(dashboard.pending_invoices ?? 0)}</b>
            </li>
            <li>
              <span>Ожидают выдачи</span>
              <b className={dashboard.provisioning_pending_subscriptions ? "warn-text" : ""}>
                {count(dashboard.provisioning_pending_subscriptions ?? 0)}
              </b>
            </li>
            <li>
              <span>Активные подписки</span>
              <b>{count(dashboard.active_subscriptions ?? 0)}</b>
            </li>
            <li>
              <span>Всего клиентов</span>
              <b>{count(dashboard.registered_users ?? 0)}</b>
            </li>
            <li>
              <span>Конверсия в оплату</span>
              <b>{conversion == null ? "—" : `${conversion}%`}</b>
            </li>
            <li>
              <span>Статус очередей</span>
              <Badge tone={attention ? "warn" : "ok"}>{attention ? "есть задачи" : "в норме"}</Badge>
            </li>
          </ul>
        </Card>
        <Card title="Статусы подписок" description="Текущее распределение">
          <RankedBars
            entries={(analytics.subscriptions_by_status || []).map((entry) => ({
              key: entry.key,
              label: label(entry.key),
              value: entry.value,
            }))}
            formatValue={count}
          />
        </Card>
        <Card title="Статусы счетов" description={`Созданные счета, ${days} дн.`}>
          <RankedBars
            entries={(analytics.invoices_by_status || []).map((entry) => ({
              key: entry.key,
              label: label(entry.key),
              value: entry.value,
            }))}
            formatValue={count}
          />
        </Card>
      </div>

      <div className="grid two">
        <Card title="Тарифы по выручке" description={`Топ предложений за ${days} дн.`}>
          <RankedBars
            entries={(analytics.top_tariffs || []).map((entry) => ({
              key: entry.code,
              label: entry.code,
              value: entry.revenue_minor,
              caption: `${count(entry.subscriptions)} подписок`,
            }))}
            formatValue={(value) => money(value)}
            emptyLabel="Тарифы ещё не создавались"
          />
        </Card>
        <Card title="Последние действия" description="Журнал привилегированных операций">
          {auditItems.length ? (
            <ul className="feed">
              {auditItems.map((item) => (
                <li key={item.id}>
                  <i className={item.actor_user_id ? "dot ok" : "dot warn"} aria-hidden="true" />
                  <span className="feed-main">
                    <b>{item.action}</b>
                    <small>{item.target_type}</small>
                  </span>
                  <span className="feed-time">{timeAgo(item.created_at)}</span>
                  <Badge tone={item.actor_user_id ? "ok" : "warn"}>{item.actor_user_id ? "админ" : "система"}</Badge>
                </li>
              ))}
            </ul>
          ) : (
            <EmptyState title="Нет действий" detail="Журнал заполняется после изменений в панели." />
          )}
        </Card>
      </div>
    </>
  );
}
