# План разработки VPN Bot v2

## 1. Цель и ограничения

Создать публичный, поддерживаемый и переносимый продукт для продажи VPN-доступа в Telegram: Rust-бэкенд, современная админ-панель и Mini App, PostgreSQL + Redis, контейнерный деплой из одной папки на сервере.

Источник функциональных требований и ориентир для переосмысления продукта: `/opt/vpn-bot`.

Ориентир по доставке контейнеров: `/opt/stardust/.github/workflows/backend.yml` и `/opt/stardust/deploy/`.

### Обязательные принципы

- Функциональный паритет с текущим ботом для P0-сценариев, без переноса старых данных и конфигурации.
- v2 запускается как чистая инсталляция: пользователи, балансы, подписки, платежи и настройки из legacy не импортируются.
- Новый репозиторий не содержит токенов, паролей, дампов БД, пользовательских подписок или настоящих доменов.
- Конфигурация окружения хранит только инфраструктурные секреты и адреса. Все не-секретные продуктовые и операционные параметры хранятся в PostgreSQL и редактируются в админке: бренд, тексты, тарифы, trial, промо/реферальные правила, каналы, статусы и доступность платёжных провайдеров, уведомления, расписания, политики подписок и отображаемые client instructions.
- Каждый сервис stateless; постоянные данные размещаются только в PostgreSQL, Redis и явно смонтированных каталогах данных.
- На production-сервере не выполняется сборка исходников: CI публикует готовые immutable-образы в GHCR, сервер делает `pull` и перезапуск Compose-стека.
- Внешние API имеют версионированные контракты, проверку входных данных, структурированные ошибки и OpenAPI-описание.
- Микросервисы выделяются по границам владения данными и независимому масштабированию, а не ради количества контейнеров. Общая бизнес-логика не дублируется между сервисами.

### Целевой результат

- Telegram-бот: продажи, пополнение баланса, покупка/продление, рефералы, промокоды, обязательные каналы, уведомления, поддержка и административные действия.
- Провижининг VPN: интеграция с Remnawave и безопасная выдача/продление подписок.
- Платежи: Crypto Pay, Telegram Stars и подключаемые провайдеры с корректной обработкой webhook-ов и идемпотентностью.
- Public API для Mini App и SPA-админки.
- Новый Mini App и админка с адаптивным UX, визуальным качеством Signal Dashboard как ориентиром для административных экранов и Alpha Wave как ориентиром для пользовательского Mini App. Референсы не копируются как код или бренд.
- Воспроизводимый production-деплой из одной директории с `docker-compose.yml`, `.env`, `data/`, `logs/`, конфигурацией reverse proxy и сертификатами.

## 2. Аудит текущей версии и карта функциональности

Перед началом реализации зафиксировать поведение `/opt/vpn-bot` в отдельном документе `docs/legacy-parity.md`.

### Инвентаризация

- [ ] Составить таблицу команд бота, callback-ов, состояний диалогов, ролей и сообщений на RU/EN.
- [ ] Описать все пользовательские сценарии: старт, обязательная подписка, смена языка, тарифы, баланс, покупка, продление, триал, реферал, промокод, поддержка, просмотр подписки и удаление аккаунта.
- [ ] Описать административные сценарии: пользователи, платежи, тарифы, промокоды, рассылки, зеркала, bypass-ссылки, отчёты, аудит и настройки бренда.
- [ ] Зафиксировать текущие интеграции и контракты: Telegram Bot API, Telegram Mini App init data, Crypto Pay, Stars, Platega, Anore, Remnawave, источники внешних подписок.
- [ ] Выявить устаревшие данные SQLite, YAML и файлов, которые не должны попадать в v2.
- [ ] Определить устаревшие функции и поля конфигурации. Для каждого указать решение: перенос, замена или удаление.

### Контракт паритета

- [ ] Для каждого сценария определить приоритет: P0 без него нельзя переключаться, P1 можно выпустить после запуска, P2 исключить из v2.
- [ ] Описать наблюдаемые правила: расчёт цены, длительности, реферального вознаграждения, пробного периода, статусов счетов и подписок.
- [ ] Создать набор фикстур из обезличенных данных и контрактные тесты, чтобы сравнивать ключевые результаты старой и новой логики.
- [ ] Отдельно проверить существующую схему авторизации Mini App. В v2 пользователь идентифицируется только по криптографически проверенному `Telegram.WebApp.initData`; идентификатор пользователя из тела запроса никогда не является источником доверия.

**Критерий готовности:** известны все P0-сценарии и их входы/выходы; решение об исключённых возможностях принято до проектирования БД.

## 3. Архитектура

### 3.1 Репозиторий и Rust workspace

Создать Cargo workspace с явным разделением приложений и общих библиотек.

```text
apps/
  api/                 # HTTP API для админки, Mini App и Telegram Bot
  telegram-bot/        # Telegram Bot (polling/webhook) — только обработка команд
  aggregator/          # агрегатор подписки (backend + frontend)
  landing/             # лендинг страница (статичный SPA)
crates/
  domain/              # бизнес-логика: модели, state machine, правила
  storage/             # PostgreSQL-репозитории и миграции
  integrations/        # Telegram, платёжные провайдеры, Remnawave
  api-contracts/       # DTO, OpenAPI-схемы, общие ошибки
  observability/       # tracing, метрики, health/readiness
web/
  admin/               # SPA админки
  mini-app/            # Telegram Mini App
  aggregator/          # SPA страницы подписки (для браузера)
deploy/
docs/
```

### 3.2 Границы сервисов

На первом production-релизе развернуть следующие процессы.

#### Backend (API)

| Сервис | Ответственность | Хранилище/интерфейсы |
| --- | --- | --- |
| `api` | REST API для всех фронтендов (Mini App, Admin, Telegram Bot), авторизация, webhook-и, выдача SPA, биллинг, провижининг, уведомления, реконсиляция | PostgreSQL, Redis, HTTP |
| `telegram-bot` | Обработка команд, callback-ов, диалогов — только бизнес-логика, без прямого доступа к БД | HTTP → API |

> **Все модули backend** (billing, provisioning, notification) — это часть `api`, а не отдельные процессы. Они взаимодействуют с БД напрямую через общие crates.

#### Агрегатор

| Сервис | Ответственность | Хранилище/интерфейсы |
| --- | --- | --- |
| `aggregator` (backend) | Обработка запросов от VPN-клиентов, трансформация данных Remnawave, отдача страниц подписки в браузере | Remnawave API, PostgreSQL |
| `aggregator` (frontend) | Страница подписки для браузера (informative) | HTTP |

**Логика агрегатора:**
1. Клиент обращается к агрегатору
2. Агрегатор определяет тип клиента: браузер или VPN-клиент
3. **Если браузер** — отдаёт информативную страницу с данными о подписке
4. **Если VPN-клиент** — идёт в Remnawave, получает данные в самом информативном формате, трансформирует под конкретный VPN-клиент и отдаёт

#### Фронтенды

| Сервис | Ответственность | Хранилище/интерфейсы |
| --- | --- | --- |
| `admin-web` | SPA админки | HTTP → API |
| `mini-app-web` | SPA Mini App в Telegram | HTTP → API |
| `landing` | Лендинг страница (статичный SPA) | HTTP |

> **Промокоды и рефералы:** обрабатываются через фронт → API. В боте все пользователи равны, промокоды/рефералы применяются при оплате.

### 3.3 Модули backend (api)

#### Доменные модули (crates/domain)

| Модуль | Описание |
| --- | --- |
| `user` | Модель пользователя, профиль, язык, баланс |
| `subscription` | Подписка: создание, продление, истечение, state machine |
| `tariff` | Тарифы: типы (подписка/трафик), длительность, цена |
| `payment` | Инвойсы, статусы, реконсиляция |
| `promo` | Промокоды: создание, валидация, применение |
| `referral` | Рефералы: программа, бонусы, статистика |
| `notification` | Шаблоны, очереди, доставка |
| `admin` | RBAC, аудит, настройки |

#### Интеграции (crates/integrations)

| Модуль | Описание |
| --- | --- |
| `telegram` | Telegram Bot API, webhook, Mini App auth |
| `remnawave` | Remnawave API client |
| `payments` | Crypto Pay, Stars, Platega, Anore — общий trait |

#### Модули api (приложение)

| Модуль | Описание |
| --- | --- |
| `billing` | Обработка платежей, webhook-и провайдеров, баланс, начисления |
| `provisioning` | Выдача/продление/блокировка подписок через Remnawave |
| `notification` | Очередь уведомлений, рассылки, напоминания |

### 3.4 Обмен событиями и надёжность

- [ ] Использовать transactional outbox в PostgreSQL для событий `payment_confirmed`, `subscription_requested`, `subscription_changed`, `notification_requested` и событий аудита.
- [x] Воркер читает события с PostgreSQL `FOR UPDATE SKIP LOCKED`, фиксирует lease, владельца, attempts, время следующей попытки и последнюю ошибку. Lease-aware claim исключает параллельную обработку после commit и позволяет восстановить задание после restart; real PostgreSQL test покрывает claim, stale-lease recovery и retry deferral.
- [ ] Все внешние операции имеют idempotency key. Повтор webhook-а, рестарт воркера или сетевой таймаут не должен дважды пополнять баланс или выдавать второй VPN-ключ.
- [ ] Redis использовать для rate limit, кратоживущих сессий, distributed lock и кэша; Redis не является источником денежных или подписочных данных.
- [ ] Отложить Kafka/RabbitMQ до подтверждённой потребности. На первом этапе PostgreSQL outbox уменьшает операционную сложность и даёт надёжную доставку.

### 3.4.1 Rate Limiting

```text
Redis-based rate limiting для всех входящих запросов:

глобальный (DDoS защита):
  ├─ лимит на IP: 100 req/min
  └─ при превышении → 429 Too Many Requests

API:
  ├─ анонимные эндпоинты: 30 req/min per IP
  ├─ авторизованные: 100 req/min per user
  ├─ платёжные webhook-и: 50 req/min per provider
  └─ admin mutations: 20 req/min per admin

бот:
  ├─ команды: 20 req/min per user
  └─ callback-и: 30 req/min per user

алгоритм: sliding window counter (Redis ZSET)
при блокировке: добавить IP в бан-лист на N секунд
```

### 3.4.2 Кэширование (Redis)

```text
кэшируемые сущности:
  ├─ тарифы: TTL 5 минут (редко меняются)
  ├─ пользователи: TTL 1 минуту (для бота —частые чтения)
  └─ настройки приложения: TTL 5 минут

паттерн: cache-aside (read-through)
  ├─ чтение: check cache → miss → DB → set cache
  ├─ запись: DB → invalidate cache
  └─ инвалидация: при изменении через админку — мгновенная

не кэшировать:
  ├─ подписки (aggregator запрашивает Remnawave напрямую)
  ├─ платежи (всегда свежие данные)
  └─ секреты (никогда в кэше)
```

### 3.4.3 Circuit Breaker и Graceful Degradation

```text
Remnawave недоступен:
  ├─ circuit breaker: 5 ошибок подряд → open → retry через 30 сек
  ├─ уведомление в Telegram: "⚠️ Remnawave недоступна"
  ├─ новые покупки: инвойс создан, provision в очередь (retry)
  ├─EXISTING подписки: работают (ноды закэшированы в aggregator)
  └─ aggregator: отдаёт кэшированные данные или ошибку

Платёжный провайдер недоступен:
  ├─ circuit breaker: 3 ошибки подряд → open → retry через 60 сек
  ├─ уведомление в Telegram: "⚠️ {provider} недоступен"
  ├─ пользователю: показать альтернативный провайдер
  └─ инвойс: остаётся pending, реконсиляция проверит позже

Redis недоступен:
  ├─ rate limiting: отключается (accept all)
  ├─ кэш: пропускается (read from DB)
  └─ сессии: JWT-based (без Redis)
  └─ НЕ критично — работает без кэша и rate limit

PostgreSQL недоступен:
  ├─ ВСЁ останавливается
  ├─ health check → container restart
  └─ НЕ допускать: мониторинг, алерты, реплики
```

### 3.4.4 Аудит (Audit Logging)

```text
все изменения в БД логируются в таблицу audit_log:

таблица audit_log:
  ├─ id (uuid)
  ├─ timestamp (timestamptz)
  ├─ user_id (uuid, nullable) — кто выполнил
  ├─ action (text) — create/update/delete/login/logout
  ├─ entity_type (text) — user/tariff/payment/promo/setting/...
  ├─ entity_id (uuid, nullable) — что изменилось
  ├─ details (jsonb) — старые/новые значения
  ├─ ip_address (inet, nullable)
  └─ user_agent (text, nullable)

что логируется:
  ├─ CRUD тарифов, промокодов, каналов
  ├─ изменение баланса, подписки, трафика
  ├─ вход/выход из админки
  ├─ изменение настроек (Remnawave, платежи, Telegram)
  ├─ создание/удаление пользователей
  └─ все платёжные операции

просмотр в админке:
  ├─ фильтр по пользователю, действию, сущности, дате
  ├─ пагинация
  └─ экспорт (опционально)
```

### 3.5 Внешний API

- [ ] Версионировать API через `/api/v1`.
- [ ] Разделить публичные Mini App маршруты, админские маршруты и webhook-маршруты по middleware/ролям.
- [x] Публиковать OpenAPI 3.1 contract через `/api/v1/openapi.json`; он описывает все текущие `/api/v1` routes, bearer/webhook boundaries, reusable DTO/error schemas, pagination/filter parameters и примеры. Contract test закрепляет ключевые операции и auth boundary.
- [ ] Пагинация, сортировка и фильтры админских списков проектируются единообразно.
- [ ] Изменения контракта проходят contract/integration tests до выпуска фронтенда.

**Критерий готовности:** процессы, владение данными, синхронные API и асинхронные события описаны в `docs/architecture.md` и реализованы без циклических зависимостей.

### 3.6 Performance Targets

```text
целевые метрики:

API:
  ├─ latency: p95 < 200ms, p99 < 500ms
  ├─ throughput: 1000+ req/s
  └─ error rate: < 0.1%

Aggregator:
  ├─ latency: p95 < 500ms (включая fetch из Remnawave)
  ├─ throughput: 500+ concurrent connections
  └─ cache hit rate: > 80% (для кэшированных данных)

Telegram Bot:
  ├─ response time: < 2s на команду
  └─ message delivery: < 5s

resources per container:
  ├─ api: 2 CPU, 512MB RAM
  ├─ telegram-bot: 1 CPU, 256MB RAM
  ├─ aggregator: 2 CPU, 512MB RAM
  ├─ postgres: 4 CPU, 2GB RAM
  └─ redis: 1 CPU, 256MB RAM
```

### 3.7 Схема работы v2 (полный флоу)

#### Агрегатор (отдельный сервис)

Агрегатор — HTTP-сервис, который обрабатывает запросы от VPN-клиентов и браузеров.

**Как в v1 получалась подписка в Remnawave (самый информативный формат):**

```text
при провижининге (provision_paid_invoice):
  1. POST/PATCH /api/users → Remnawave создаёт/обновляет пользователя
  2. Remnawave возвращает subscriptionUrl — это URL для подключения
  3. subscriptionUrl содержит ВСЮ информацию о нодах в raw формате:
     - vless://uuid@host:port?... (каждая нода — отдельная строка)
     - vmess://base64(json)...
     - trojan://...
  4. Этот URL сохраняется в vpn_accounts.access_url
  5. Это САМЫЙ ИНФОРМАТИВНЫЙ формат — содержит все ноды, протоколы, параметры
```

**Логика агрегатора при запросе:**

```text
GET /sub/{user_id}?token={hmac_slug}

аутентификация:
  ├─ hmac_sha256(bot_token, str(user_id))[:20] == token
  └─ constant-time comparison

определение клиента (is_browser_request):
  ├─ нет User-Agent → браузер
  ├─ VPN маркеры (karing, happ, hiddify, clash, sing-box, v2ray...) → VPN клиент
  ├─ HEAD метод → VPN клиент
  ├─ User-Agent содержит mozilla/chrome/safari → браузер
  └─ Accept: text/html → браузер

если БРАУЗЕР:
  ├─ загрузить данные подписки из БД (user_id → access_url, traffic, expires)
  ├─ отрисовать HTML страницу:
  │   ├─ ID пользователя
  │   ├─ URL подписки
  │   ├─ использовано трафика
  │   ├─ срок действия
  │   ├─ deep links для Happ/INCY
  │   └─ инструкция по импорту
  └─ Content-Type: text/html

если VPN КЛИЕНТ:
  ├─ определить профиль клиента (client_profile):
  │   ├─ base64: nekoray, nekobox, v2rayN, hiddify, shadowrocket, loon, sing-box (default)
  │   ├─ xray_json: Happ, v2rayNG, Streisand, INCY
  │   ├─ singbox_json: Karing
  │   └─ clash_yaml: Clash, ClashMeta, Clash-Verge, FlClash, Mihomo, Stash
  │
  ├─ получить raw подписку из БД (access_url = subscriptionUrl от Remnawave):
  │   ├─ это САМЫЙ ИНФОРМАТИВНЫЙ формат — содержит все ноды в raw виде
  │   ├─ fetch(access_url) с заголовками:
  │   │   User-Agent: Happ/4.7.4/ios/2604141220584
  │   │   Accept: text/plain,*/*
  │   │   ↑ Этот User-Agent заставляет Remnawave отдать полный список нод
  │   │     (vless://..., vmess://...) вместо редиректа или упрощённого формата
  │   └─ если access_url недоступен → вернуть ошибку
  │
  ├─ опционально: загрузить внешние источники (external_sources) и объединить
  │
  ├─ объединить все источники, дедуплицировать ноды
  │
  ├─ отрендерить в нужном формате для конкретного клиента:
  │   ├─ base64 → base64-encoded lines (vless://..., vmess://...) — simplest format
  │   ├─ xray_json → xray config JSON { outbounds: [...], routing: {...} }
  │   ├─ singbox_json → sing-box config JSON { outbounds: [...] }
  │   └─ clash_yaml → Clash YAML proxies: [...]
  │
  ├─ auto-select (если включён):
  │   ├─ xray: balancer outbound с leastLoad/leastPing
  │   └─ sing-box: urltest outbound с probe
  │
  ├─ direct route для российских доменов (.ru, .su, yandex, vk...)
  │
  └─ response headers:
      ├─ Profile-Title: base64(brand_name)
      ├─ Subscription-Userinfo: upload=0; download={used}; total={limit}; expire={unix}
      ├─ Profile-Update-Interval: 3600
      ├─ Support-Url, Profile-Web-Page-Url
      └─ Content-Type: text/plain | application/json | text/yaml
```

#### Remnawave провижининг (в api)

```text
provision(invoice, tariff, user):
  ├─ найти пользователя в Remnawave: GET /api/users/by-telegram-id/{telegram_id}
  ├─ если существует:
  │   ├─ expires_at = max(now, current_expiry) + tariff.duration
  │   ├─ traffic_limit += tariff.traffic_bytes (если докупка трафика)
  │   └─ PATCH /api/users { uuid, expireAt, trafficLimitBytes }
  ├─ если не существует:
  │   ├─ expires_at = now + tariff.duration
  │   ├─ POST /api/users { username, expireAt, trafficLimitBytes, activeInternalSquads, ... }
  │   └─ получить uuid
  ├─ получить subscriptionUrl от Remnawave (содержит raw ноды)
  └─ сохранить subscriptionUrl в vpn_accounts.access_url
      ↑ это ТОТ САМЫЙ URL, который агрегтор будет раздавать VPN-клиентам
        и показывать на странице подписки в браузере

extend(telegram_id, delta_hours):
  └─ текущий expiry + delta_hours → ensure_user_until

add_traffic(telegram_id, bytes):
  └─ текущий лимит + bytes → set_traffic_limit

revoke(telegram_id):
  └─ expires_at = now → ensure_user_until

delete(telegram_id):
  └─ DELETE /api/users/{uuid}
```

## 4. Данные и PostgreSQL

### 4.1 Модель данных

Спроектировать схему как SQL-артефакты для внешнего контролируемого применения. Минимальные группы таблиц:

- [ ] `users`, `user_profiles`, `user_languages`, `user_settings`, `roles`, `admin_sessions`.
- [ ] `brands`, `brand_locales`, `app_settings`, `media_assets`, `legal_documents`.
- [ ] `tariffs`, `tariff_prices`, `promo_codes`, `promo_redemptions`, `referrals`, `referral_rewards`.
- [ ] `wallets`, `wallet_transactions`, `invoices`, `payment_attempts`, `payment_webhook_events`, `payout_allocations`.
- [ ] `vpn_accounts`, `subscriptions`, `subscription_events`, `vpn_provider_credentials`, `provider_sync_jobs`.
- [ ] `required_channels`, `external_subscription_sources`, `bypass_links`, `mirrors`.
- [ ] `notifications`, `broadcasts`, `notification_deliveries`, `scheduled_reports`.
- [ ] `outbox_events`, `idempotency_keys`, `audit_log`.

### 4.2 Правила целостности

- [ ] Деньги хранятся в целых минимальных единицах валюты (`amount_minor`), не в `float`.
- [ ] Баланс изменяется только проводками `wallet_transactions`; текущее значение либо вычисляется, либо поддерживается в транзакции с журналом изменений.
- [ ] Для счетов, подписок и webhook-ов определить конечные статусы, разрешённые переходы и уникальные ключи провайдера.
- [ ] Внешние идентификаторы (`telegram_user_id`, invoice ID провайдера, Remnawave ID) имеют уникальные ограничения там, где это требуется.
- [ ] Soft delete использовать только для сущностей, которым нужна историчность; персональные данные при удалении аккаунта анонимизировать по заранее согласованной политике.
- [ ] Все изменения из админки фиксируются в `audit_log` с актором, объектом, diff и корреляционным идентификатором.

### 4.3 Настройки и брендинг

- [ ] В `.env` оставить: строки подключения, ключи шифрования, токены интеграций, разрешённые публичные URL, режим окружения и параметры логирования.
- [ ] В БД хранить: название, описание, логотипы, цветовые токены, контакты поддержки, публичные ссылки, тексты, локализации, тарифы, каналы, включённые функции, настройки уведомлений и расписаний.
- [ ] Для каждого нового не-секретного параметра добавлять admin API, валидацию, audit log, безопасное default значение и UI-состояния загрузки/ошибки/сохранения; не создавать отдельные environment toggles для продуктовой логики.
- [ ] Секреты интеграций шифровать прикладным ключом из environment; интерфейс никогда не возвращает значение секрета после сохранения.
- [ ] Исторические брендовые значения для уже созданного инвойса/подписки сохранять снимком, если они отображаются в документах или сообщениях.

### 4.4 Чистая инициализация

- [x] On an empty PostgreSQL volume, Compose applies `db/init/001_baseline.sql` to create the P0 schema without users, payments, subscriptions, or secrets. API initializes default trial/referral settings at startup; bootstrap administrator remains environment allowlist-based until first-run role UI is added.
- [ ] Подготовить seed-механизм исключительно для development/staging с синтетическими данными и тестовыми интеграциями.
- [ ] В production начальные тарифы, бренд, платёжные интеграции и каналы настраиваются через админку после первого входа.
- [ ] Не импортировать SQLite, YAML, пользовательские записи, старые сессии, токены, VPN-ключи и любые legacy-секреты.

**Критерий готовности:** чистый production-стенд разворачивается с пустой БД, bootstrap-доступом администратора и без зависимости от файлов legacy-проекта.

## 5. Доменные модули

### 5.1 Пользователи и Telegram

- [ ] Авторизация Mini App: проверка подписи `initData`, TTL `auth_date`, защита от replay в пределах согласованного окна, серверная сессия/короткоживущий JWT с ротацией.
- [ ] Админские права: allowlist Telegram ID для bootstrap, затем роли и разрешения в БД; критичные действия требуют повторной проверки сессии.
- [ ] Поддержка RU/EN с локализацией в БД/файлах ресурсов, fallback-стратегией и возможностью менять контент из админки.
- [ ] Явная state machine для многошаговых диалогов бота вместо неявного состояния обработчиков.
- [x] Ограничение частоты API-запросов и антиспам для Mini App login, admin mutations и payment webhook-ов через Redis. Telegram init-data replay key устанавливается атомарно через Redis `SET NX EX`; opt-in real Redis test подтверждает replay rejection и TTL. Telegram commands/callbacks остаются задачей bot transport.

### 5.2 Каталог, баланс и покупки

- [ ] Тарифы имеют активность, порядок, продолжительность, цену, валюту, описание, метки и правила доступности.
- [ ] Покупка выполняется одной транзакцией: проверка цены/доступности, списание, создание заказа/подписки, outbox-событие для провижининга.
- [ ] Повторный запрос клиента возвращает прежний результат по ключу идемпотентности.
- [ ] Неуспешный провижининг переводит заказ в понятный промежуточный статус и автоматически запускает повтор/компенсацию по политике, определённой до запуска.
- [ ] Реферальные и промо-правила считаются на сервере и снабжаются журналом начислений/погашений.

### 5.3 Платежи

- [x] Платёжные провайдеры реализуются как подключаемые адаптеры единого контракта: создание счёта, нормализация суммы/валюты, получение статуса, проверка webhook-а и нормализация события оплаты. Коммерческая логика, проводки, идемпотентность и outbox не зависят от конкретного провайдера.
- [x] Реестр провайдеров формируется из включённых конфигураций. Провайдер можно включить/выключить без изменения кода коммерческого потока; отключение запрещает новые счета, но не отменяет reconciliation уже созданных счетов до их terminal status.
- [x] Первый набор адаптеров: Crypto Pay, Anore и Telegram Stars. Новые адаптеры добавляются отдельным модулем, fixture-тестами и настройкой в PostgreSQL; секреты остаются только в environment/secret storage.
- [x] Адаптеры выполняются в `billing-worker`, а не в отдельном контейнере по умолчанию. Отдельный процесс допускается лишь при доказанной потребности в независимом масштабировании, SLA или изоляции сбоя; HTTP webhook-ingestion, ledger и outbox сохраняют единое владение состоянием.
- [ ] Каждый provider adapter обязан безопасно обрабатывать retries/timeouts, иметь provider-side idempotency reference, не логировать платёжные секреты и принимать webhook только после provider-specific signature/timestamp validation. Для источников без подписываемого webhook-а используются доверенный Telegram update и reconciliation, а не неподтверждённый публичный callback.
- [ ] Реализовать единый интерфейс платёжного провайдера: создать счёт, проверить статус, обработать webhook, отменить/истечь, нормализовать сумму и валюту.
- [ ] Первым реализовать провайдеры, реально используемые в legacy production; остальные переносить после P0.
- [ ] Webhook проверяет подпись, timestamp и идентификатор события; сырой payload сохраняется с ограниченной ретенцией и редактированием секретов в логах.
- [ ] Сверка незавершённых счетов запускается по расписанию и не конфликтует с webhook-обработкой.
- [x] Повторный `paid` webhook не меняет баланс и не создаёт дополнительную проводку. Real PostgreSQL handler test подписанного Anore webhook подтверждает одну запись webhook, payment attempt и direct-purchase subscription при replay.

### 5.4 VPN-провижининг и подписки

- [ ] Изолировать Remnawave за портом/адаптером, чтобы доменная модель не зависела от API-полей провайдера.
- [ ] Поддерживаемый контракт Remnawave зафиксирован на legacy API v2, который использует текущий адаптер: `GET /api/users/by-telegram-id/{telegramId}`, `POST/PATCH /api/users`, `POST /api/users/{uuid}/actions/disable` и `DELETE /api/users/{uuid}`. Адаптер использует Bearer API token. Remnawave Panel v3.0.0, выпущенный в июле 2026, заменил UUID на numeric user ID и удалил lookup по Telegram ID, поэтому production-панель должна быть закреплена на v2-совместимой версии до отдельной миграции адаптера. Официальные источники: `https://docs.rw/sdk/go-sdk/`, `https://docs.rw/sdk/python-sdk/`, `https://f.docs.rw/t/topic/354`.
- [x] Реализовать операции создания, продления и блокировки: истечение локальной подписки создаёт идемпотентное outbox-задание `subscription.block_requested`, а provisioning worker повторяет `disable` до подтверждения Remnawave. `404` считается успешным достижением целевого состояния.
- [x] Реализовать adapter-level удаление Remnawave account через `DELETE /api/users/{uuid}`. Пользовательское/админское удаление аккаунта и retention policy остаются отдельным P0 API-сценарием, чтобы не удалять доступ без явного подтверждения.
- [x] Реализовать периодическую read-only сверку provider status, срока действия и traffic usage через `GET /api/users/{uuid}`. Сверка фиксирует в `subscription_events` provider state, отсутствие аккаунта и расхождение срока, но не изменяет локальную коммерческую подписку автоматически.
- [ ] Добавить устойчивое хранилище traffic snapshots/retention policy, затем включать пользовательские usage-уведомления.
- [ ] Хранить внешний ID и subscription URL защищённо; не писать URL с токенами в application logs, ошибки или audit diff.
- [ ] Определить политику истечения, grace period, трафиковых лимитов, squad-ов и повторного использования аккаунта.
- [ ] **P0, критично: адаптивный subscription gateway.** Воссоздать ключевой сценарий legacy: персональный защищённый endpoint определяет клиент по `User-Agent`/`Accept`, получает самый информативный доступный upstream payload, нормализует узлы и рендерит конфигурацию, совместимую с этим клиентом. Поддержать: Xray JSON для Happ/v2rayNG/Streisand/INCY, sing-box JSON для Karing, Clash YAML для Clash-family, Base64 URI как безопасный fallback. Для browser-запроса этот же URL отдаёт subscription landing page: статус/срок доступа, traffic при наличии, инструкции по поддерживаемым клиентам, copy/import action и безопасные deep-link trampolines; не отдаёт конфиг вместо HTML. Ссылка доступна только при активной entitlement, не раскрывает upstream URLs/credentials и не логируется.
- [x] Базовый gateway: Mini App получает защищённую `/sub/{subscription_id}?token=...` ссылку вместо raw Remnawave URL; token HMAC-bound к subscription ID, endpoint проверяет active entitlement, upstream требует HTTPS, не следует redirects, ограничивает размер/timeout и отвечает `Cache-Control: no-store`. Browser requests получают server-rendered landing page с состоянием и гайдом; non-browser clients пока получают наиболее информативный Happ-formatted upstream payload до завершения renderers.
- [x] Базовая client adaptation: payload декодируется из Base64 или Xray JSON, VLESS/Hysteria2 nodes deduplicate, а client profile выбирает Base64 fallback, Xray JSON, sing-box JSON или Clash YAML.
- [x] Client renderers сохраняют transport/TLS/Reality поля (`ws`, `grpc`, `xhttp`, `splithttp`, SNI, uTLS fingerprint, Reality public key/short ID, Hysteria2 obfs/congestion); unit fixtures закрепляют Reality-over-WebSocket и Hysteria2 contracts.
- [ ] Automated renderer contract fixtures cover Happ/v2rayNG Xray JSON, Karing sing-box JSON, and Mihomo/Clash YAML selection plus Reality/Hysteria2 field preservation. Before production, manual staging import with an anonymized current Remnawave payload is still required for the target client versions.
- [ ] Gateway получает основной payload из Remnawave subscription URL. Внешние источники, static nodes, bypass pool и source labels остаются вторичным механизмом расширения node set, а не целью P0; если включаются, URL шифруются, fetch защищён от SSRF/redirect to private network, cookies/Authorization не пересылаются, есть response-size/timeout/cache limits.
- [ ] Вынести client profiles, node normalization и renderers в отдельный модуль/воркер. После базовой совместимости перенести auto-select: Xray observatory/balancer с двухуровневым bypass fallback и sing-box urltest group. Не включать bypass policy без отдельного продуктового и юридического решения.

### 5.5 Уведомления и отчёты

- [x] Все исходящие сообщения создаются как задания в БД, а не отправляются внутри HTTP-транзакции. `notification-worker` materializes `notification.requested`, доставляет Telegram сообщения, применяет exponential backoff и завершает permanent failures.
- [ ] Реализовать шаблоны уведомлений с локализацией и безопасными параметрами.
- [x] Добавить базовые P0 статусы: wallet top-up, referral reward, subscription active и subscription expired. Напоминания, provisioning failure alerts, административные оповещения и ежедневные отчёты остаются следующими типизированными уведомлениями.
- [ ] Рассылки имеют сегмент, предпросмотр, ограничение скорости, отмену и статистику доставки.
- [ ] Реализовать admin-configurable automation rules: включение, условие/сегмент, задержка, cooldown, лимит аудитории, локализованный шаблон, действие, preview/dry-run и audit log.
- [ ] Первый automation trigger: `subscription_expired_without_renewal` — подписка истекла N дней назад, у пользователя нет новой активной или ожидающей выдачи подписки. Условие не называется "пользовался VPN", пока нет доверенного источника usage/session telemetry от Remnawave.
- [ ] Первый automation action: персонализированное Telegram-уведомление через существующую очередь. Постановка должна быть идемпотентной на `(rule_id, user_id, subscription_id)` и не создавать повторов при retry/restart.
- [ ] Promo action допускается только после появления server-enforced связи промокода с конкретным пользователем, лимита активаций, срока действия и атомарного журналирования выдачи. Нельзя отправлять в Telegram код, который может использовать другой пользователь.
- [ ] Новые triggers/actions добавляются как типизированные серверные варианты, а не как исполняемые пользователем скрипты; raw SQL, шаблоны с доступом к секретам и произвольные HTTP actions запрещены.

### 5.6 Оставшийся P0 объём

- [x] Telegram bot navigation layer: polling принимает messages/callback queries; `/start` создаёт профиль и реферал, `/menu`, `/plans`, `/trial`, `/access`, `/language`, `/support` и inline callbacks работают через stateless PostgreSQL checks. Required-channel gate использует `getChatMember`, deep links `/start plan|buy` и `/start access` ведут к нужной задаче, а checkout/access import остаются в Mini App вместо дублирования коммерческой логики в Bot API.
- [x] Telegram transport: production webhook mode регистрирует HTTPS `/telegram/webhook` с Telegram secret token, endpoint делает constant-time verification, а reverse proxy передаёт запрос только внутреннему bot service. Polling mode при запуске снимает webhook, поэтому режимы переключаются без конфликтующего `getUpdates`. Команды регистрируются через `setMyCommands`.
- [x] Telegram bot: локализованный `/help`, registered commands и P0 notification handoff доступны.
- [x] Добавить bot-focused HTTP/integration tests: mock Telegram API проверяет recovery polling после `500` и payload `setMyCommands`; real PostgreSQL test подтверждает, что Telegram Stars update replay создаёт только одну запись webhook, payment attempt и direct-purchase subscription. Проверка Telegram webhook secret остаётся unit boundary существующего transport.
- [ ] Commerce: промокоды, рефералы, правила продления, server-side price snapshots и понятная compensation policy для невосстановимого provisioning failure.
- [ ] Admin: CRUD тарифов, пользователей, подписок, платежей, promos/referrals, brand/content, channels, notification rules, audit log и system status.
- [ ] **P0, критично: адаптивный subscription gateway.** Персональный endpoint получает наиболее информативный Remnawave payload, распознаёт subscription client и отдаёт Xray JSON, sing-box JSON, Clash YAML либо Base64 URI fallback. Browser request к этому же URL отдаёт landing page подписки с гайдами, доступом и safe import/deep-link actions. Нормализация, deduplication, client-compatible rendering и token/entitlement checks обязательны; внешние источники не являются базовым P0-потоком.
- [ ] Remnawave: продление, блокировка, удаление, периодическая сверка, traffic/status synchronization и политика grace period.
- [x] Локальный lifecycle expiry: worker переводит истёкшую `active` подписку в `expired`, пишет событие и ставит notification и provider block через outbox. Блокировка у Remnawave повторяется до успешного подтверждения.
- [x] API: опубликован OpenAPI 3.1 с DTO/error contracts и contract tests; единый runtime error body (`code`, `message`) применяется также к admin routes. Владелец может получить детали одного invoice через `GET /api/v1/invoices/{id}`; admin users/subscriptions/invoices/audit используют server-side pagination and filters.
- [ ] Security: rate limits, CORS allowlist, admin session reauthentication, audit views, dependency/secret scanning и webhook payload retention/redaction.
- [ ] Deployment: reverse proxy TLS, immutable image tags, production Compose, first-run flow, backups/restore, rollback и runbooks.
- [ ] Validation: integration tests с PostgreSQL/Redis/mock Telegram/payment/Remnawave покрывают payment/Stars replay, Telegram polling recovery, provisioning lease recovery и mock Remnawave activation-to-notification path. Локальный Compose публикует PostgreSQL/Redis только на `127.0.0.1` для opt-in tests. `db/init/001_baseline.sql` passed a clean-database smoke run covering API/payment, provisioning lease, storage and Telegram Stars tests. Full external staging E2E `Telegram -> payment -> subscription -> Remnawave -> notification` remains required before release because no staging credentials/payload are present.

**Критерий готовности:** P0-поток "Telegram -> оплата -> баланс -> покупка -> Remnawave -> подписка -> уведомление" покрыт интеграционными тестами и устойчив к повторам запросов.

## 6. Frontend и UX/UI

### 6.1 Общие правила

- [ ] Отдельные SPA для `admin` и `mini-app`; общий пакет дизайн-токенов, иконок, API-клиента и базовых компонентов только при реальной общей потребности.
- [ ] Адаптивность от 320 px, управление с клавиатуры, достаточный контраст, состояния загрузки/ошибки/пустого списка и локализация являются частью acceptance criteria.
- [ ] Не переносить legacy UI постранично. Сначала описать пользовательские задачи и информационную архитектуру.
- [ ] Минимизировать передачу чувствительных данных в браузер; авторизация и полномочия всегда проверяются API.

### 6.2 Админка

- [ ] Взять за ориентир композицию, плотность данных, графики и удобство Signal Dashboard, не копируя исходные ассеты, тексты или брендинг.
- [ ] Главный экран: выручка, платежи, активные подписки, новые пользователи, конверсия, ошибки провижининга и сравнительные периоды.
- [ ] Разделы: пользователи, подписки, платежи, тарифы, промокоды, рефералы, рассылки, интеграции, бренд/контент, аналитика, аудит и системное состояние.
- [ ] Раздел Automation: список правил, статус, trigger/action, сегмент, cooldown, последняя обработка, preview аудитории, dry-run, результаты доставки и audit history.
- [ ] Таблицы: серверная пагинация, сохранённые фильтры, экспорт с ограничением прав и понятные статусы.
- [ ] Критичные операции: подтверждение, понятные последствия, аудит и защита от двойного submit.
- [ ] Графики строятся по агрегированным API, поддерживают выбранный период и не загружают сырые большие наборы данных в браузер.

### 6.3 Mini App

- [ ] Пересобрать с нуля вокруг трёх основных действий: состояние подписки, выбор/покупка тарифа, получение/копирование инструкции подключения.
- [ ] Визуальный язык вдохновляется Alpha Wave: ясная иерархия, выразительная типографика, аккуратная анимация и mobile-first, но без заимствования кода и фирменного оформления.
- [ ] Экран подписки показывает состояние, срок, трафик при наличии, ссылку и инструкции для клиентов.
- [ ] Экран оплаты показывает доступный баланс, цену, выбранный тариф и окончательный результат операции.
- [ ] Глубокие ссылки из Telegram-бота открывают соответствующий экран Mini App.
- [ ] Проверить Telegram light/dark theme, safe areas, offline/error flow и открытие вне Telegram.

**Критерий готовности:** дизайн утверждён до реализации, обе SPA проходят e2e P0-сценарии в мобильном viewport и не имеют блокирующих accessibility-проблем.

## 7. Безопасность

- [ ] До публикации репозитория провести secret scan всей истории. Скомпрометированные токены отозвать и выпустить заново; не ограничиваться удалением из текущего файла.
- [ ] Добавить `.env.example`, `config.example` только с плейсхолдерами, `.gitignore`, pre-commit/CI secret scanning и документацию по локальным секретам.
- [ ] Пароли и ключи не попадают в логи, метрики, trace, ошибки API, скриншоты и экспорт админки.
- [ ] Использовать Argon2id для локальных секретов/паролей, HMAC constant-time compare для подписей, CSPRNG для токенов.
- [ ] Включить CSRF-защиту для cookie-based admin-сессий, корректные `Secure`, `HttpOnly`, `SameSite` cookie-флаги и allowlist CORS origins.
- [ ] Webhook-и принимать только по HTTPS, проверять подписи и применять rate limit. Для Telegram использовать secret token webhook-а, если выбран webhook-режим.
- [ ] Админские действия защищены RBAC, audit log и ограничением частоты; доступ к служебным endpoint-ам не публикуется наружу.
- [x] `deploy/backup-postgres.sh` creates PostgreSQL custom-format dumps and `deploy/restore-rehearsal.sh` restores into a temporary database without replacing production. The rehearsal passed locally on July 24, 2026. Encryption, off-host storage, and retention remain production operational requirements.
- [ ] Добавить dependency audit и базовый SAST в CI; исправления критичных уязвимостей блокируют выпуск.

**Критерий готовности:** проведён threat model для платежей, Mini App, админки и webhook-ов; нет известных активных секретов в Git и нет критичных замечаний security review.

## 8. Контейнеризация и production-директория

### 8.1 Структура на сервере

```text
/opt/vpn-bot-v2/deploy/
  docker-compose.yml
  .env
  db/
    init/
  data/
    postgres/
    redis/
  backups/
  host-nginx.example.conf
  update.sh
  rollback.sh
```

### 8.2 Docker Compose

- [x] Сервисы: `postgres`, `redis`, `api`, `telegram-bot`, `billing-worker`, `provisioning-worker`, `notification-worker`, `admin-web`, `mini-app-web`. Reverse proxy не запускается в Compose.
- [x] PostgreSQL и Redis не публикуют порты на host; внутреннее взаимодействие идёт по выделенной Compose-сети.
- [x] API, Mini App, Admin и Telegram webhook публикуются только на `127.0.0.1`. Host administrator configures nginx, DNS and TLS separately; Docker receives neither host nginx configuration nor certificate files.
- [ ] Добавить `healthcheck`, `depends_on` с readiness БД, `restart: unless-stopped`, лимиты ресурсов и ротацию логов.
- [ ] Использовать теги образов по SHA для production-релиза; `latest` допустим только для development или явно выбранного канала.
- [x] Создать self-contained `deploy/.env.example`, `deploy/docker-compose.yml`, `deploy/db/init`, `deploy/host-nginx.example.conf`, `deploy/update.sh`, `deploy/rollback.sh` и `docs/production-runbook.md`. Runtime data and local backups stay under `deploy/`; rollback меняет только image SHA и не удаляет persistent volumes.

### 8.3 Эксплуатация

- [x] Описать первичную установку, обновление и rollback по immutable SHA в `docs/production-runbook.md`. Update/rollback scripts verify API readiness; rollback validates the SHA and avoids dynamic shell evaluation. Backup/restore rehearsal, key rotation и emergency provider-disable остаются эксплуатационными процедурами до staging credentials.
- [x] Рестарт одного worker не теряет задания благодаря lease-aware PostgreSQL outbox claim and stale-lock recovery. Integration tests cover payment replay, active/stale provisioning leases, and mock Remnawave provisioning through activation and notification outbox enqueue.
- [ ] Проверить переносимость: запуск тестового стенда путём копирования только `deploy/` directory and supplying its `.env`; host nginx/TLS are recreated by the server administrator.

**Критерий готовности:** чистый сервер запускает систему без локальной сборки, а восстановление PostgreSQL backup в отдельный стенд подтверждено документированным тестом.

## 9. CI/CD и качество

### 9.1 Pull request CI

- [ ] Rust: `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`, `cargo deny` или эквивалентный audit.
- [ ] API: OpenAPI contract tests and integration tests with PostgreSQL and Redis service containers.
- [ ] Frontend: lockfile-based install, typecheck, lint, unit tests, production build.
- [ ] E2E: минимум P0-scenarios на staging-like Compose-стеке с mock Telegram/payment/Remnawave endpoints.
- [ ] Security: secret scanning, dependency scanning, Docker image scan.
- [ ] Проверять форматирование Compose, Dockerfile и отсутствие секретов в артефактах.

### 9.2 Сборка и публикация

- [ ] По модели Stardust: одна job компилирует Rust workspace и запускает тесты; готовые бинарные артефакты передаются matrix-job для упаковки сервисных образов без повторной компиляции в Docker.
- [ ] SPA собираются в отдельных multi-stage Docker image и отдаются nginx/caddy как статические файлы.
- [ ] Публиковать образы в GHCR с immutable тегом SHA, тегом релиза и каналом (`staging`/`stable`) при необходимости.
- [ ] Генерировать SBOM и сохранять test/build artifacts на ограниченный срок.

### 9.3 Деплой

- [ ] Для staging: автоматический деплой после merge в основную ветку или отдельный staging-канал.
- [ ] Для production: ручное подтверждение релизного SHA или protected environment GitHub Actions.
- [ ] CI передаёт только изменившиеся compose/nginx файлы, затем сервер выполняет `docker compose pull`, `up -d --remove-orphans` и health check.
- [ ] При неуспешной проверке здоровья запускать documented rollback на предыдущий SHA, не удаляя volume с данными.
- [ ] Нотифицировать о результате deploy в закрытый административный канал без секретов.

**Критерий готовности:** каждый merge проходит полный набор проверок, а релиз с конкретным SHA можно развернуть и откатить воспроизводимо.

## 10. Наблюдаемость и поддержка

- [ ] Структурированные JSON-логи с `request_id`, `user_id` в обезличенной/безопасной форме, `invoice_id`, `subscription_id` и `trace_id` при наличии.
- [ ] `/healthz` для liveness и `/readyz` для готовности зависимостей у HTTP-сервисов.
- [ ] Метрики: HTTP latency/error rate, webhook failures, очередь outbox, payment conversion, provisioning latency/failures, Telegram delivery failures, PostgreSQL pool и Redis availability.
- [ ] Дашборд для операционных метрик и алерты на backlog, ошибки платежей, ошибки выдачи подписок, недоступность БД/Redis и истекающие сертификаты.
- [ ] Политика ретенции логов, audit payload и webhook payload; персональные данные не удерживаются дольше необходимого.
- [ ] Runbooks: платеж завис в pending, подписка не создана, Remnawave недоступен, Telegram API rate limit, восстановление БД, отзыв скомпрометированного секрета.

## 11. Архитектура и схема работы legacy VPN Bot v1

### 11.1 Стек технологий

| Компонент | Технология |
| --- | --- |
| Язык | Python 3.12+ |
| Telegram-фреймворк | aiogram 3.x (polling) |
| Хранение данных | SQLite (`bot.db`) |
| Конфигурация | YAML (`config.yml` + `config.local.yml`) |
| VPN-провайдер | Remnawave API v2 |
| Платежи | Crypto Pay API, Telegram Stars, Platega, Anore |
| HTTP-сервер | aiohttp (aggregator, admin panel) |
| Контейнеризация | Docker + docker-compose |

### 11.2 Структура проекта

```text
src/vpn_sales_bot/
  bot.py               # точка входа: инициализация, фоновые задачи, graceful shutdown
  config.py            # dataclass-модель конфигурации, парсинг YAML
  db/                  # SQLite-репозитории (users, invoices, vpn_keys, referrals, promo_codes)
  handlers/
    __init__.py        # регистрация всех handler-групп в Dispatcher
    menu.py            # /start, навигация, язык, настройки, подписки
    tariffs.py         # просмотр тарифов, покупка за баланс, промокоды, триал
    payments.py        # пополнение баланса, прямая покупка, Stars, webhook/poll
    admin.py           # административные команды
    mirrors.py         # зеркала ботов
    common.py          # общие хелперы (cabinet_text, has_active_vpn, preferred_subscription_url)
    states.py          # FSM-состояния (TariffStates, TopUpStates)
  services/
    context.py         # BotContext — контейнер зависимостей для всех handlers
  payments.py          # CryptoBotClient, PlategaClient, AnoreClient
  vpn.py               # VpnProvider (RemnawaveVpnProvider, MockVpnProvider)
  aggregator/          # HTTP-сервис для VPN-клиентов и browser landing pages
    api/               # REST-эндпоинты
    renderers/         # Xray JSON, sing-box JSON, Clash YAML, Base64
    pages/             # server-rendered HTML subscription pages
    services/          # business logic
    transforms/        # нормализация, deduplication нод
  admin_panel.py       # admin SPA + runtime config applier
  notifications.py     # отправка уведомлений в Telegram
  i18n.py              # локализация RU/EN
  keyboards.py         # inline-клавиатуры
  text.py              # рендеринг текстов
  mirrors.py           # менеджер зеркал
  backups.py           # бэкап SQLite в Telegram
```

### 11.3 Инициализация и жизненный цикл

```text
main()
  ├─ load_config(config.yml)           # парсинг YAML → AppConfig
  ├─ Database(db_path).init()          # создание таблиц SQLite
  ├─ build_payments_client()           # CryptoBotClient (mock или live)
  ├─ build_platega_client()            # PlategaClient
  ├─ build_anore_client()              # AnoreClient
  ├─ build_vpn_provider()              # RemnawaveVpnProvider или MockVpnProvider
  ├─ Bot(token) → get_me()
  ├─ build_dispatcher(config, db, ...)
  │   ├─ BotContext(...)               # контейнер зависимостей
  │   ├─ AggregatorService(...)        # HTTP-сервис
  │   └─ register_all(dp, ctx)         # регистрация handlers
  ├─ mirror_manager.start_existing()   # запуск зеркал
  ├─ [aggregator HTTP server]          # aiohttp на listen_port
  ├─ [admin panel HTTP server]         # aiohttp на admin_port
  └─ [background tasks]
      ├─ dp.start_polling(bot)         # основной polling Telegram
      ├─ _run_db_backup_loop()         # бэкап SQLite → Telegram (1ч)
      ├─ _run_subscription_notifications_loop()  # напоминания (1ч)
      ├─ _run_traffic_notifications_loop()       # трафик-лимиты (30мин)
      ├─ _run_provider_invoice_poll_loop()       # опрос статусов invoices (15с)
      └─ _run_daily_report_loop()      # ежедневный отчёт
```

### 11.4 Пользовательские flow

#### Регистрация и старт

```text
/start [ref_{referrer_id}]
  ├─ upsert_user(user_id, username)
  ├─ set_user_mirror_if_empty(user_id)
  ├─ ensure_subscription_message()     # проверка подписки на обязательные каналы
  │   └─ get_chat_member() для каждого канала (кэш 60с)
  ├─ set_referrer_if_empty()           # привязка реферала
  └─ render_welcome() → главное меню
```

#### Триал (72ч, 10 ГБ трафика)

```text
trial:start
  ├─ ensure_subscription_callback()
  ├─ user_lock(user_id)               # защита от гонок
  ├─ user_has_ever_had_vpn_access()    # проверка:เคย ли был VPN
  ├─ user_has_used_trial()             # проверка: был ли триал
  ├─ create_invoice(trial, amount=0)
  ├─ set_invoice_status("paid")
  ├─ ensure_min_traffic_limit(10 GB)
  ├─ vpn_provider.provision()          # → Remnawave POST/PATCH /api/users
  │   ├─ _get_user(telegram_id)        # GET /api/users/by-telegram-id/{id}
  │   ├─ _create_user() или _update_user()
  │   └─ возврат subscriptionUrl
  ├─ save_vpn_key(invoice, access_key, expires_at)
  ├─ set_user_access_flags(has_vpn_access=True)
  └─ set_user_unified_access_url()
```

#### Покупка тарифа за баланс

```text
tariff:buy:{code}
  ├─ validate_discount_promo_code()    # если есть промокод
  ├─ charge_user_balance(final_price)
  ├─ create_invoice(tariff, amount, purpose_code)
  ├─ set_invoice_status("paid")
  ├─ provision_paid_invoice()
  │   ├─ product == "vpn": vpn_provider.provision()
  │   ├─ product == "subscription": vpn_provider.update_expiration()
  │   └─ product == "traffic": vpn_provider.set_traffic_limit()
  ├─ save_vpn_key()
  ├─ set_user_access_flags()
  └─ notify_payment_success()
```

#### Прямая покупка тарифа (внешний платёж)

```text
directpay:{provider}:{code}
  ├─ create invoice у провайдера
  │   ├─ CryptoPay: create_amount_invoice() → PaymentLink
  │   ├─ Platega: create_invoice() → PaymentLink
  │   ├─ Anore: create_invoice() → PaymentLink
  │   └─ Stars: send_invoice() → Telegram native checkout
  ├─ create_invoice(in DB, provider_invoice_id)
  └─ ожидание оплаты
      ├─ pre_checkout_query (Stars)
      ├─ successful_payment (Stars) → handle_paid_direct_purchase()
      ├─ deposit:check:{id} (ручная проверка)
      ├─ webhook от Platega/Anore
      └─ poll loop (CryptoPay/Platega/Anore, каждые 15с)
          └─ apply_paid_invoice()
              ├─ mark_invoice_fulfillment_applied()
              ├─ redeem_invoice_promo_code()
              └─ provision_paid_invoice()
```

#### Пополнение баланса

```text
menu:topup → выбор суммы → topup:{provider}:{amount}
  ├─ CryptoPay: create_amount_invoice() → deposit invoice
  ├─ Stars: create invoice → send_invoice()
  ├─ Platega: create_invoice() → deposit invoice
  └─ Anore: create_invoice() → deposit invoice
      └─ после оплаты:
          ├─ mark_invoice_balance_applied()
          ├─ add_user_balance()
          ├─ apply_referral_bonus() → реферальный бонус (%)
          └─ notify_payment_success()
```

### 11.5 Платёжные провайдеры

#### Единый контракт (payments.py)

```python
class CryptoBotClient:
    async def create_amount_invoice(amount_rub, user_id, description) -> PaymentLink
    async def get_invoice_status(provider_invoice_id) -> str
    def is_invoice_paid(status) -> bool

class PlategaClient:
    async def create_invoice(amount_rub, user_id, description) -> PaymentLink
    async def get_invoice_status(invoice_id) -> str
    def is_paid(status) -> bool

class AnoreClient:
    async def create_invoice(amount_rub, user_id, description) -> PaymentLink
    async def get_invoice_status(invoice_id) -> str
    def is_paid(status) -> bool
    def verify_webhook_signature(raw_body, signature) -> bool
```

#### PaymentLink

```python
@dataclass
class PaymentLink:
    invoice_id: str              # ID инвойса у провайдера
    bot_invoice_url: str         # URL для оплаты
    mini_app_invoice_url: str    # URL для Mini App
    web_app_invoice_url: str     # URL для WebApp
```

#### Статусы invoice

| Статус | Описание |
| --- | --- |
| `active` | Ожидает оплаты |
| `paid` | Оплачен, выполняется provisioning |
| `expired` | Истёк срок оплаты |
| `failed` | Ошибка обработки |
| `cancelled` | Отменён (rollback) |

### 11.6 VPN-провижининг (Remnawave API v2)

#### Контракты

```python
class VpnProvider:
    async def provision(invoice, tariff, username, traffic_limit_bytes) -> ProvisionedAccess
    async def get_user(telegram_id) -> dict | None
    async def update_expiration(telegram_id, delta_hours) -> ProvisionedAccess
    async def ensure_user_until(telegram_id, expires_at, username) -> ProvisionedAccess
    async def set_traffic_limit(telegram_id, traffic_limit_bytes) -> ProvisionedAccess
    async def revoke(telegram_id) -> ProvisionedAccess
    async def delete_user(telegram_id) -> None
    async def health_status() -> VpnHealthStatus
```

#### Remnawave API endpoints

| Операция | Метод | Endpoint |
| --- | --- | --- |
| Получить пользователя | GET | `/api/users/by-telegram-id/{telegramId}` |
| Создать пользователя | POST | `/api/users` |
| Обновить пользователя | PATCH | `/api/users` |
| Удалить пользователя | DELETE | `/api/users/{uuid}` |
| Список squad-ов | GET | `/api/internal-squads` |

#### Payload создания/обновления

```json
{
  "username": "vpn_{telegram_id}",
  "expireAt": "2026-01-01T00:00:00Z",
  "status": "ACTIVE",
  "trafficLimitBytes": 10737418240,
  "trafficLimitStrategy": "NO_RESET",
  "activeInternalSquads": ["uuid1", "uuid2"],
  "externalSquadUuid": "uuid3",
  "tag": "user_tag",
  "telegramId": 123456,
  "description": "username"
}
```

#### ProvisionedAccess

```python
@dataclass
class ProvisionedAccess:
    access_key: str     # subscriptionUrl из Remnawave
    expires_at: str     # ISO 8601 UTC
```

#### Логика provision

```python
def provision():
  existing = _get_user(telegram_id)
  if existing is None:
    expire_at = now + tariff.duration_hours
    user = _create_user(payload)
  else:
    expire_at = existing.expireAt + tariff.duration_hours  # продление
    user = _update_user(user_uuid, payload)
  return user.subscriptionUrl
```

### 11.7 Агрегатор (subscription gateway)

#### Назначение
- Генерация защищённого URL для VPN-клиентов
- Нормализация и deduplication нод из Remnawave
- Рендеринг конфигов для различных клиентов
- Browser-запросы → subscription landing page

#### Структура

```text
aggregator/
  api/                # REST: /sub/{user_id}, /api/v1/invoices, webhook endpoints
  renderers/          # client-specific renderers
    xray.py           # Xray JSON (Happ, v2rayNG, Streisand, INCY)
    singbox.py        # sing-box JSON (Karing)
    clash.py          # Clash YAML (Clash-family, Mihomo)
    base64.py         # Base64 URI fallback
  pages/              # server-rendered HTML landing pages
  services/           # business logic
    client_profile.py # выбор renderer по User-Agent/Accept
  transforms/         # нормализация нод, dedup, фильтрация
  models.py           # dataclass-ы
  facade.py           # единый входной point
  request_classifier.py  # определение клиента по заголовкам
```

#### Client detection

```text
Request → request_classifier.py
  ├─ User-Agent содержит "Happ" → Xray JSON
  ├─ Accept: application/json + platform "Karing" → sing-box JSON
  ├─ User-Agent содержит "Clash" / "Mihomo" → Clash YAML
  └─ fallback → Base64 URI
```

#### URL subscription

```text
/sub/{user_id}?token={hmac_slug}
  ├─ Проверка token (HMAC bound to user_id)
  ├─ Проверка active entitlement
  ├─ Получение payload из Remnawave subscription URL
  ├─ Нормализация нод (dedup по address:port)
  └─ Рендеринг по client profile
```

### 11.8 Модель данных (SQLite)

#### Основные таблицы

| Таблица | Назначение |
| --- | --- |
| `users` | telegram_id, username, balance_rub, language, has_vpn_access, unified_access_url |
| `user_settings` | notification preferences, mirrors |
| `invoices` | user_id, tariff_code, amount_rub, status, provider_invoice_id, purpose_code |
| `vpn_keys` | user_id, invoice_id, access_key, expires_at, tariff_code |
| `referrals` | referrer_id, referred_id |
| `referral_rewards` | referrer_id, invoice_id, amount_rub |
| `promo_codes` | code, promo_type, discount_percent, amount_rub, max_uses, allowed_tariffs |
| `promo_redemptions` | user_id, promo_code, invoice_id |
| `traffic_notifications` | user_id, notification_type, sent_at |
| `subscription_notifications` | user_id, product, notification_type, expires_at, sent_at |
| `stars_charges` | charge_id, invoice_id, user_id, amount |

#### Ключевые связи

```text
users.telegram_id ──< invoices.user_id
invoices.invoice_id ──< vpn_keys.invoice_id
users.telegram_id ──< referrals.referrer_id (FK: users.telegram_id)
users.telegram_id ──< vpn_keys.user_id
invoices.invoice_id ──< referral_rewards.invoice_id
```

### 11.9 Фоновые задачи

| Задача | Интервал | Описание |
| --- | --- | --- |
| `payment_poll` | 15 сек | Опрос активных инвойсов у CryptoPay/Platega/Anore, обработка статусов |
| `subscription_notify` | 1 час | Уведомления об истечении подписки (3 дня / истекла) |
| `traffic_notify` | 30 мин | Уведомления о трафике (5/3/1 ГБ) |
| `daily_report` | по расписанию | Ежедневная сводка в заданный час (MSK) |
| `db_backup` | 1 час | Бэкап SQLite → Telegram chat |

### 11.10 Telegram Bot API

#### Команды

| Команда | Описание |
| --- | --- |
| `/start` | Регистрация, привязка реферала, главное меню |
| `/start ref_{id}` | Регистрация с реферальным кодом |
| `/lang`, `/language` | Смена языка |

#### Callback flow

```text
menu:start       → главное меню
menu:buy         → список тарифов
menu:balance     → баланс
menu:topup       → ввод суммы
menu:myvpn       → мой доступ (ключ, трафик)
menu:trial       → триал
menu:referrals   → реферальная программа
menu:support     → поддержка
menu:info        → информация
menu:settings    → настройки
menu:language    → выбор языка
```

### 11.11 Конфигурация (config.yml)

```yaml
bot:
  token: "..."
  admin_ids: [123456]

brand:
  name: "VPN Service"
  support_username: "@support"
  bot_username: "@vpn_bot"

referral:
  enabled: true
  reward_percent: 10

payments:
  provider: "cryptobot"
  mode: "live"
  asset: "USDT"
  payment_timeout_minutes: 30
  stars:
    enabled: true
    rate_per_rub: 1.0
  platega:
    enabled: true
    base_url: "https://app.platega.io"
    merchant_id: "..."
    secret: "..."
  anore:
    enabled: true
    base_url: "https://api.anore.cc/v1"
    api_key: "..."
    secret_key: "..."
    webhook_secret: "..."

tariffs:
  monthly:
    title: "1 Month"
    duration_hours: 720
    price_rub: 500
    description: "VPN access for 30 days"
    product: "vpn"
    traffic_gb: 0

vpn:
  delivery_mode: "remnawave"
  key_prefix: "vpn"
  remnawave:
    enabled: true
    base_url: "https://panel.example.com"
    api_token: "..."
    traffic_limit_bytes: 10737418240
    traffic_limit_strategy: "NO_RESET"
    internal_squad_uuids: ["uuid1"]
    external_squad_uuid: "uuid2"
    user_tag: "regular"
    username_prefix: "vpn"

aggregator:
  enabled: true
  public_base_url: "https://sub.example.com"
  listen_host: "0.0.0.0"
  listen_port: 8088
  auto_select: true

required_channels:
  - chat_id: -100123456789
    url: "https://t.me/channel"
```

### 11.12 Известные ограничения legacy

| Ограничение | Описание |
| --- | --- |
| SQLite | Нет параллельного доступа, нет транзакций уровня PostgreSQL |
| Нет outbox | Платёжные операции выполняются синхронно в HTTP-обработчике |
| Нет идемпотентности | Повтор webhook-а может создать дубликат проводки |
| Нет state machine | Статусы invoice/подписки неявные, переходы не ограничены |
| Нет audit log | Действия администратора не логируются |
| Нет RBAC | Админские права — только allowlist по Telegram ID |
| Файловая конфигурация | Тарифы, каналы, настройки хранятся в YAML, не в БД |
| Нет API-контракта | Нет OpenAPI, нет contract tests |
| Нет health checks | Нет `/healthz`/`/readyz` |
| Монолит | Все компоненты в одном процессе |

## 12. Тестирование

- [ ] Unit: доменные правила, state machine, цены, реферальные расчёты, промокоды, статусы счёта и подписки.
- [ ] Repository: PostgreSQL tests for constraints and transactions.
- [ ] Integration: адаптеры Crypto Pay/Stars/Remnawave на mock HTTP-серверах, проверка повторов и сетевых ошибок.
- [ ] Contract: API-схемы для обоих фронтендов и webhook-провайдеров.
- [ ] E2E: пользовательская покупка, повтор webhook-а, продление, истечение, админское редактирование тарифа и рассылка.
- [ ] Load: целевые проверки на burst Telegram updates, webhook-и и массовую рассылку до production запуска.
- [ ] Manual acceptance: мобильные Telegram iOS/Android, desktop Telegram, современные браузеры, RU/EN, light/dark theme.

## 13. Установка и развёртывание

### 13.1 Сценарий установки

Установка выполняется одной командой на чистый сервер:

```bash
sudo sh setup.sh
```

Скрипт:
1. Скачивает `deploy/` из GitHub (docker-compose.yml, миграции, скрипты)
2. Создаёт структуру директорий
3. Генерирует `postgres_password` через openssl
4. Создаёт `.env` с портами и секретами
5. Запускает `docker compose pull && docker compose up -d`

### 13.2 Структура на сервере

```text
/opt/vpn-bot-v2/
  docker-compose.yml
  .env                          # secrets + порты
  db/
    init/
      001_baseline.sql          # схема P0
    migrations/
      manifest.txt
      *.sql
  data/
    postgres/                   # volume
    redis/                      # volume
  backups/
  update.sh
  rollback.sh
  backup-postgres.sh
  restore-rehearsal.sh
  apply-runtime-settings.sh
  host-nginx.example.conf
```

### 13.3 Автоинициализация БД

При старте API выполняется проверка:

```text
startup()
  ├─ SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'users')
  │   ├─ true → пропуск
  │   └─ false → APPLY 001_baseline.sql
  ├─ SELECT EXISTS (SELECT 1 FROM app_settings WHERE key = 'trial_settings')
  │   ├─ true → пропуск
  │   └─ false → INSERT trial_settings (72ч, 10 ГБ)
  ├─ SELECT EXISTS (SELECT 1 FROM app_settings WHERE key = 'referral_settings')
  │   ├─ true → пропуск
  │   └─ false → INSERT referral_settings (10%)
  └─ bootstrap admin:
      ├─ ADMIN_TELEGRAM_IDS из .env
      ├─ если пользователь с таким telegram_user_id нет → создать с ролью admin
      └─ если есть → пропуск
```

### 13.4 Отказ от шифрования в БД

- **Не шифровать:** `vpn_accounts.access_url` хранится открытым текстом
- **Не шифровать:** платёжные секреты хранятся в `app_secrets` открытым текстом (доступ только к PostgreSQL)
- **APPLICATION_ENCRYPTION_KEY** — удаляется из `.env` и кода
- Причина: шифрование на уровне приложения добавляет сложность без реальной выгоды, если PostgreSQL доступен только из внутренней сети

### 13.5 Архитектура сервисов

| Сервис | Порт (host) | Назначение |
| --- | --- | --- |
| `api` | 18080 | REST API для Mini App и админки |
| `telegram-bot` | — | Telegram polling/webhook (внутренний) |
| `billing-worker` | — | Reconciliation invoices (внутренний) |
| `provisioning-worker` | — | Remnawave + outbox (внутренний) |
| `notification-worker` | — | Доставка уведомлений (внутренний) |
| `admin-web` | 18082 | SPA админки (static files) |
| `mini-app-web` | 18081 | SPA Mini App (static files) |
| `postgres` | 127.0.0.1:5432 | PostgreSQL |
| `redis` | 127.0.0.1:6379 | Redis |

### 13.6 Reverse proxy (nginx/caddy)

Пользователь самостоятельно настраивает reverse proxy с TLS (nginx, caddy и т.д.) на основе примера ниже:

```nginx
# admin.example.com → admin panel + API
server {
    listen 443 ssl http2;
    server_name admin.example.com;

    ssl_certificate /etc/letsencrypt/live/admin.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/admin.example.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:18082;
    }

    location /api/ {
        proxy_pass http://127.0.0.1:18080;
    }
}

# app.example.com → mini app + API
server {
    listen 443 ssl http2;
    server_name app.example.com;

    location / {
        proxy_pass http://127.0.0.1:18081;
    }

    location /api/ {
        proxy_pass http://127.0.0.1:18080;
    }
}

# landing.example.com → landing page
server {
    listen 443 ssl http2;
    server_name landing.example.com;

    location / {
        proxy_pass http://127.0.0.1:18084;
    }
}
```

### 13.7 Первичная настройка (first-run)

После установки и настройки rproxy пользователь:

1. Заходит в админку по `https://admin.example.com`
2. Создаёт первого root-администратора (полные права)
3. Проходит пошаговую настройку:

#### Шаг 1: Сервер и домены

| Сервис | Описание | Локальный порт | Публичный хост |
| --- | --- | --- | --- |
| API | REST API для Mini App и админки | 18080 | `api.example.com` |
| Admin Panel | SPA админки | 18082 | `admin.example.com` |
| Mini App | SPA Mini App в Telegram | 18081 | `app.example.com` |
| Landing | Лендинг страница | 18084 | `landing.example.com` |
| Subscription Aggregator | Страница подписки / VPN конфиги | 18085 | `sub.example.com` |
| Telegram Webhook | Webhook URL (опционально, если не polling) | 18083 | `hook.example.com` |

- **Кнопка «Сохранить»** — перезапускает контейнеры
- Все настройки (токены, секреты, тарифы) хранятся только в БД, не в `.env`

#### Шаг 2: Telegram Bot

| Параметр | Описание |
| --- | --- |
| Bot Token | Токен от @BotFather |
| Bot Username | Получается автоматически из Bot API |
| Webhook URL | Публичный адрес для Telegram (заполняется автоматически из домена) |

> **Роль в боте:** Все пользователи равны. Роль admin — только в админке. Админом становится тот, у кого есть учётная запись в админке с ролью root или admin.

Сохраняются в `app_secrets` (только в БД, не в `.env`).

#### Шаг 3: Remnawave (VPN провайдер)

| Параметр | Описание |
| --- | --- |
| API URL | `https://remnawave.example.com` |
| API Token | Bearer token для авторизации |
| Squad UUID | UUID сквада, который выдаётся пользователям |

- **Squad UUID** — пользователи попадают в указанный сквад при выдаче подписки
- **Агрегатор** — модифицирует запрос/ответ Remnawave для совместимости с VPN-клиентом (трансформация формата, добавление/уборка полей)
- Сохраняются в `app_secrets` (только в БД, не в `.env`)

#### Шаг 4: Платёжные системы

Модульный принцип: каждый провайдер — отдельный блок, который можно включить/выключить.

| Провайдер | Параметры | По умолчанию |
| --- | --- | --- |
| Crypto Pay | Bot Token | откл. |
| Telegram Stars | Bot Token (общий с Telegram Bot) | откл. |
| Platega | Shop ID, Secret Key | откл. |
| Anore | API Key, Secret Key | откл. |

- Каждый провайдер имеет on/off toggle
- Секреты вводятся через админку, хранятся только в `app_secrets` (БД)
- При отображении в админке — маскируются (`****`), при редактировании — в открытом виде
- Добавление нового провайдера: создать модуль в коде, подключить

#### Шаг 5: Тарифы

Два типа тарифов:

**1. Подписка (основная)**

| Параметр | Описание |
| --- | --- |
| Название | Отображаемое имя |
| Длительность | Срок действия (дни) |
| Трафик | Объём трафика (ГБ), 0 = без ограничений |
| Цена | Стоимость в рублях |
| Статус | Активен / Неактивен |
| Порядок сортировки | Порядок отображения в боте |

**2. Докупка трафика**

| Параметр | Описание |
| --- | --- |
| Название | Отображаемое имя |
| Трафик | Объём докупаемого трафика (ГБ) |
| Цена | Стоимость в рублях |
| Статус | Активен / Неактивен |

- Любой тариф можно оплатить любым активным платёжным провайдером

#### Шаг 6: Уведомления и алерты

Места доставки уведомлений — чаты/группы/ЛС, куда отправляются уведомления.

| Место | Тип | Описание |
| --- | --- | --- |
| Chat ID | int | ID чата/группы/канала |
| Topic ID | int, nullable | ID топика (для супергрупп с включёнными темами) |
| Тип уведомлений | multiselect | Какие уведомления сюда отправляются |

Типы уведомлений:
  - О платежах (успех/ошибка)
  - О Remnawave (недоступна, ошибка provision)
  - О платёжных провайдерах (недоступен, ошибка webhook)
  - Ежедневный отчёт
  - О трафике (мало осталось)
  - О подписках (истекает)
  - Аудит (важные действия)

При добавлении места:
  1. Ввести Chat ID
  2. Если группа с топиками — ввести Topic ID (опционально)
  3. Выбрать какие уведомления сюда отправляются
  4. Тестовое уведомление для проверки

- Сохраняются в `notification_targets` (БД)
- По умолчанию: ни одно место не настроено

После сохранения:
- Перезапускаются затронутые контейнеры через `docker compose up -d`
- API автоматически обновляет webhook URL в Telegram (если настроен)
- Бот и платёжные системы начинают работать

### 13.8 Обновление

```bash
cd /opt/vpn-bot-v2
sudo ./update.sh
```

Скрипт:
1. Определяет текущий SHA из running container
2. Делает `docker compose pull`
3. Применяет миграции (если есть новые)
4. Делает `docker compose up -d --remove-orphans`
5. Проверяет health API

### 13.9 Откат

```bash
cd /opt/vpn-bot-v2
sudo ./rollback.sh <previous-sha>
```

Откат меняет только image tags, не удаляет volume.

### 13.10 Бэкап и восстановление

```bash
# Бэкап
sudo ./backup-postgres.sh

# Восстановление (в отдельный стенд)
sudo ./restore-rehearsal.sh
```

Политика бэкапов:
  ├─ частота: раз в день (cron, 03:00 MSK)
  ├─ хранилище: локальная папка `/opt/vpn-bot-v2/backups/`
  ├─ retention: последние 3 версии (старые удаляются автоматически)
  ├─ формат: PostgreSQL custom-format dump (pg_dump -Fc)
  ├─ именование: `vpn_bot_YYYYMMDD_HHMMSS.dump`
  ├─ проверка целостности: pg_restore --list после создания
  └─ будущее: S3 (Яндекс) для off-site хранения

## 14. Этапы реализации

### Этап 0. Discovery и решения

- [x] Выполнить аудит legacy и утвердить P0/P1/P2.
- [x] Утвердить threat model, модель данных, контракты интеграций и UX-прототипы.
- [x] Зафиксировать решение о Telegram polling либо webhook и перечень платёжных провайдеров первого релиза.

**Выход:** `docs/legacy-parity.md`, `docs/architecture.md`, ERD, API-черновик и UX-прототипы.

### Этап 1. Основание платформы

- [x] Создать Rust workspace, базовые crates, конфигурацию, error handling, tracing, Dockerfiles и локальный Compose.
- [x] Поднять PostgreSQL/Redis, health/readiness, RBAC-каркас, audit/outbox.
- [x] Настроить PR CI, image build и GHCR publish без production deploy.

**Выход:** пустой, но наблюдаемый и тестируемый стек запускается одной командой локально и в staging.

### Этап 2. P0 backend

- [x] Реализовать пользователей, Telegram auth, настройки/брендинг, каталог тарифов и кошелёк.
- [ ] Реализовать первый платёжный провайдер, invoices/webhooks/reconciliation и идемпотентность.
- [ ] Реализовать Remnawave adapter, покупку, продление, выдачу подписки, outbox-воркеры и критичные уведомления.
- [ ] Реализовать адаптивный subscription gateway: защищённый URL, entitlement, browser subscription landing page с гайдами, client detection, получение наиболее информативного Remnawave payload, node normalization и Xray/sing-box/Clash/Base64 renderers.

**Выход:** API поддерживает полный P0 коммерческий поток в staging.

### Этап 3. Telegram-бот и интерфейсы

- [x] Реализовать P0-диалоги Telegram-бота: `/start`, меню, RU/EN, обязательные каналы, deep links, тарифы, trial, access status, Mini App handoff и support entrypoint.
- [ ] Реализовать Bot API checkout только при подтверждённой необходимости: он не должен дублировать уже авторитетные Mini App invoice/wallet flows без общего use case/contract tests.
- [ ] Реализовать новый Mini App и админку для P0-сценариев.
- [ ] Добавить аналитический dashboard, управление тарифами/пользователями/платежами/настройками, обязательными каналами и поддержкой.

**Выход:** пользователь и оператор могут пройти P0-сценарии без legacy UI.

### Этап 4. Подготовка production

- [ ] Поднять чистый staging и провести e2e, regression, load и security checks на синтетических данных.
- [ ] Проверить first-run flow: bootstrap администратора, настройку бренда, тарифов, платёжного провайдера и Remnawave через админку.
- [ ] Утвердить release SHA, rollback decision point, ответственных и runbook запуска.

**Выход:** production-runbook для чистой инсталляции и staging без неразрешённых P0-дефектов.

### Этап 5. Первый production-запуск

- [ ] Развернуть v2 с пустой БД и выполнить первоначальную настройку через админку.
- [ ] Проверить платежи, выдачу, Telegram updates, метрики, алерты и логи в первые часы после запуска.
- [ ] При сбое откатить только версии сервисов по immutable SHA; persistent volumes не удалять.

**Выход:** v2 обслуживает production-трафик как самостоятельная система.

### Этап 6. После запуска

- [ ] Перенести P1: дополнительные платёжные провайдеры, расширенная аналитика, дополнительные источники/агрегатор подписок, улучшения рассылок.
- [ ] Устранить наблюдаемые узкие места, сократить технический долг, обновить документацию и провести postmortem запуска.
- [ ] Архивировать legacy-инфраструктуру отдельно по её собственной политике хранения данных; v2 от неё не зависит.

## 15. Риски и решения до начала разработки

| Риск | Митигирование |
| --- | --- |
| Неявные правила в legacy-коде | Discovery, таблица parity и контрактные тесты до переписывания |
| Повтор webhook-а или timeout платёжного API | idempotency keys, уникальные ограничения, transactional outbox и reconciliation |
| Двойная выдача/потеря подписки при сбое Remnawave | state machine, внешний idempotency key, периодическая сверка, retry/compensation |
| Утечка секретов при публикации репозитория | отзыв секретов, очистка истории при необходимости, secret scan, environment-only secrets |
| Избыточная сложность микросервисов | ограниченное число процессов, общий domain crate, PostgreSQL outbox вместо отдельного брокера на старте |
| Недоступность Telegram/платёжного провайдера | очереди, retries с backoff, graceful degradation, операторские runbooks |
| Ошибка первичной настройки production | first-run checklist, staging rehearsal, immutable образ, backup/restore rehearsal и rollback сервисов по SHA |

## 16. Definition of Done для первого production-релиза

- [ ] Все P0-сценарии реализованы, задокументированы и проходят automated e2e tests.
- [ ] Чистая инсталляция проходит first-run checklist без зависимости от legacy-данных или конфигурации.
- [ ] Mini App auth, admin RBAC, webhook signature validation, rate limiting и audit log прошли security review.
- [ ] PostgreSQL backup восстановлен в тестовом окружении; runbooks проверены на практике.
- [ ] CI строит, тестирует, сканирует и публикует образы; production сервер не собирает код.
- [ ] Production-развёртывание и rollback по SHA проверены на staging.
- [ ] В серверной директории достаточно `docker-compose.yml`, `.env`, `data/`, `logs/`, nginx-конфигурации и сертификатов для переноса системы.
- [ ] Репозиторий пригоден для публикации: документация, license, contribution/security policy, example config, отсутствие секретов и персональных данных.
