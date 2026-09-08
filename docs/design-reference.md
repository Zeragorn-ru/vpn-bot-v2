# VECTOR preview design reference

Статический preview-релиз зафиксирован как визуальный baseline для дальнейших переделок интерфейса.

## Preview

- URL: `https://test.zeragorn.xyz`
- Mini App: `/mini`
- Admin UI: `/admin`
- Subscription aggregator: `/sub/demo`
- Health: `/healthz`

## Source of truth

Текущий reference находится в `web/preview/` и запускается через `deploy/preview-compose.yml`.
Это самостоятельная demo-витрина: она не подключается к Telegram, PostgreSQL, Redis, Remnawave или платёжным провайдерам.

## Visual language

- тёмная графитовая база;
- lime — основной action/status accent;
- muted cyan — network/subscription data;
- restrained violet — secondary state;
- строгая плоская композиция с тонкими borders;
- небольшие радиусы, минимальные shadows и controlled gradients;
- uppercase metadata labels + крупная condensed-like headline typography;
- route-map с узлами `AMS / 01` и `YOU` как фирменный network motif;
- mobile-first Mini App, desktop sidebar Admin, focused aggregator page.

## Interactive baseline

- Mini App: copy URL, connection sheet, plans sheet, support sheet, переход в aggregator;
- Admin: sidebar navigation, toast feedback, queue/health/aggregator actions;
- Aggregator: выбор Clash/Mihomo, sing-box, Xray и Generic, copy URL.

При следующем изменении UI сначала сравнивать его с этим baseline, чтобы случайно не потерять текущую иерархию, плотность и responsive-поведение.
