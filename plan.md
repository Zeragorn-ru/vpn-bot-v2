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
  api/                 # HTTP API для админки и Mini App
  telegram-bot/        # polling/webhook Telegram и пользовательские диалоги
  billing-worker/      # платежи, webhooks, outbox, периодические задачи
  provisioning-worker/ # Remnawave и жизненный цикл VPN-подписок
  notification-worker/ # уведомления, отчёты, рассылки
crates/
  domain/              # сущности, правила и use cases без HTTP/Telegram/SQL
  storage/             # PostgreSQL-репозитории и миграции
  integrations/        # Telegram, платёжные провайдеры, Remnawave
  api-contracts/       # DTO, OpenAPI-схемы, общие ошибки
  observability/       # tracing, метрики, health/readiness
web/
  admin/               # SPA админки
  mini-app/            # Telegram Mini App
deploy/
docs/
```

Рекомендуемая базовая реализация: `tokio`, `axum`, `sqlx`, `serde`, `tracing`, `tower-http`, `reqwest`, `teloxide` или эквивалентная тонкая Telegram-обвязка. Окончательный набор библиотек выбирается после короткого технического прототипа; версии фиксируются в `Cargo.lock`.

### 3.2 Границы сервисов

На первом production-релизе развернуть следующие процессы. Они могут находиться в одном workspace и использовать общие crates, но не должны обмениваться внутренними HTTP-вызовами для синхронных бизнес-операций.

| Компонент | Ответственность | Хранилище/интерфейсы |
| --- | --- | --- |
| `api` | REST API, Telegram Mini App auth, админская авторизация, выдача SPA, webhook-входы | PostgreSQL, Redis, HTTP |
| `telegram-bot` | Команды, callback-и, диалоги, отправка сообщений | PostgreSQL, Redis, Telegram API |
| `billing-worker` | Счета, сверка платежей, обработка webhook-ов, баланс, начисления | PostgreSQL, Redis, payment APIs |
| `provisioning-worker` | Создание, продление, блокировка и синхронизация VPN-подписок | PostgreSQL, Remnawave API |
| `notification-worker` | Очередь уведомлений, рассылки, напоминания, ежедневные отчёты | PostgreSQL, Redis, Telegram API |
| `admin-web` | Статическая SPA и reverse proxy к API | HTTP |
| `mini-app-web` | Статическая SPA Mini App | HTTP |

### 3.3 Обмен событиями и надёжность

- [ ] Использовать transactional outbox в PostgreSQL для событий `payment_confirmed`, `subscription_requested`, `subscription_changed`, `notification_requested` и событий аудита.
- [x] Воркер читает события с PostgreSQL `FOR UPDATE SKIP LOCKED`, фиксирует lease, владельца, attempts, время следующей попытки и последнюю ошибку. Lease-aware claim исключает параллельную обработку после commit и позволяет восстановить задание после restart; real PostgreSQL test покрывает claim, stale-lease recovery и retry deferral.
- [ ] Все внешние операции имеют idempotency key. Повтор webhook-а, рестарт воркера или сетевой таймаут не должен дважды пополнять баланс или выдавать второй VPN-ключ.
- [ ] Redis использовать для rate limit, краткоживущих сессий, distributed lock и кэша; Redis не является источником денежных или подписочных данных.
- [ ] Отложить Kafka/RabbitMQ до подтверждённой потребности. На первом этапе PostgreSQL outbox уменьшает операционную сложность и даёт надёжную доставку.

### 3.4 Внешний API

- [ ] Версионировать API через `/api/v1`.
- [ ] Разделить публичные Mini App маршруты, админские маршруты и webhook-маршруты по middleware/ролям.
- [x] Публиковать OpenAPI 3.1 contract через `/api/v1/openapi.json`; он описывает все текущие `/api/v1` routes, bearer/webhook boundaries, reusable DTO/error schemas, pagination/filter parameters и примеры. Contract test закрепляет ключевые операции и auth boundary.
- [ ] Пагинация, сортировка и фильтры админских списков проектируются единообразно.
- [ ] Изменения контракта проходят contract/integration tests до выпуска фронтенда.

**Критерий готовности:** процессы, владение данными, синхронные API и асинхронные события описаны в `docs/architecture.md` и реализованы без циклических зависимостей.

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

## 11. Тестирование

- [ ] Unit: доменные правила, state machine, цены, реферальные расчёты, промокоды, статусы счёта и подписки.
- [ ] Repository: PostgreSQL tests for constraints and transactions.
- [ ] Integration: адаптеры Crypto Pay/Stars/Remnawave на mock HTTP-серверах, проверка повторов и сетевых ошибок.
- [ ] Contract: API-схемы для обоих фронтендов и webhook-провайдеров.
- [ ] E2E: пользовательская покупка, повтор webhook-а, продление, истечение, админское редактирование тарифа и рассылка.
- [ ] Load: целевые проверки на burst Telegram updates, webhook-и и массовую рассылку до production запуска.
- [ ] Manual acceptance: мобильные Telegram iOS/Android, desktop Telegram, современные браузеры, RU/EN, light/dark theme.

## 12. Этапы реализации

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

## 13. Риски и решения до начала разработки

| Риск | Митигирование |
| --- | --- |
| Неявные правила в legacy-коде | Discovery, таблица parity и контрактные тесты до переписывания |
| Повтор webhook-а или timeout платёжного API | idempotency keys, уникальные ограничения, transactional outbox и reconciliation |
| Двойная выдача/потеря подписки при сбое Remnawave | state machine, внешний idempotency key, периодическая сверка, retry/compensation |
| Утечка секретов при публикации репозитория | отзыв секретов, очистка истории при необходимости, secret scan, environment-only secrets |
| Избыточная сложность микросервисов | ограниченное число процессов, общий domain crate, PostgreSQL outbox вместо отдельного брокера на старте |
| Недоступность Telegram/платёжного провайдера | очереди, retries с backoff, graceful degradation, операторские runbooks |
| Ошибка первичной настройки production | first-run checklist, staging rehearsal, immutable образ, backup/restore rehearsal и rollback сервисов по SHA |

## 14. Definition of Done для первого production-релиза

- [ ] Все P0-сценарии реализованы, задокументированы и проходят automated e2e tests.
- [ ] Чистая инсталляция проходит first-run checklist без зависимости от legacy-данных или конфигурации.
- [ ] Mini App auth, admin RBAC, webhook signature validation, rate limiting и audit log прошли security review.
- [ ] PostgreSQL backup восстановлен в тестовом окружении; runbooks проверены на практике.
- [ ] CI строит, тестирует, сканирует и публикует образы; production сервер не собирает код.
- [ ] Production-развёртывание и rollback по SHA проверены на staging.
- [ ] В серверной директории достаточно `docker-compose.yml`, `.env`, `data/`, `logs/`, nginx-конфигурации и сертификатов для переноса системы.
- [ ] Репозиторий пригоден для публикации: документация, license, contribution/security policy, example config, отсутствие секретов и персональных данных.
