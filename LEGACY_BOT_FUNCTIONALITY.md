# Полное описание функционала `/opt/vpn-bot`

> Документ составлен по исходному коду, конфигурации, frontend и тестам проекта `/opt/vpn-bot`. Это описание фактически реализованного поведения старой версии, а не целевая спецификация новой системы. Значения тарифов и настройки приведены по текущему `config.yml` и могут изменяться конфигурацией.

## 1. Назначение и архитектура

Проект — Telegram-бот для продажи VPN-доступа и трафика. В состав входят:

- Telegram-бот на `aiogram 3`;
- SQLite persistence;
- интеграция с Remnawave или mock VPN provider;
- платёжные провайдеры CryptoBot, Platega, Anore и Telegram Stars;
- персональные subscription URLs и агрегатор конфигураций;
- Telegram Mini App на React/Vite;
- Telegram Admin Panel;
- отдельная Web Admin Panel;
- система рефералов, промокодов и mirror-ботов;
- фоновые задачи уведомлений, polling платежей, backups и отчётов.

Главные точки входа:

| Область | Файлы |
|---|---|
| Запуск и фоновые циклы | `src/vpn_sales_bot/bot.py` |
| Telegram handlers | `src/vpn_sales_bot/handlers/` |
| Клавиатуры и тексты | `src/vpn_sales_bot/keyboards.py`, `i18n.py`, `text.py` |
| База данных | `src/vpn_sales_bot/db/` |
| Платежи | `src/vpn_sales_bot/payments.py`, `handlers/payments.py` |
| VPN provisioning | `src/vpn_sales_bot/vpn.py` |
| Subscription aggregator | `src/vpn_sales_bot/aggregator/` |
| Telegram admin | `handlers/admin.py` |
| Web admin | `admin_panel.py`, `admin_static/` |
| Mini App | `frontend/src/` |
| Конфигурация | `config.py`, `config.yml` |

Все Telegram handlers подключаются через `src/vpn_sales_bot/handlers/__init__.py`.

---

## 2. Пользовательский Telegram-сценарий

### `/start`

При запуске бота пользователь:

1. получает очищенное FSM-состояние;
2. создаётся или обновляется в таблице `users`;
3. связывается с mirror-ботом, если вход выполнен через зеркало;
4. проходит проверку обязательных каналов;
5. может быть привязан к рефереру через параметр `ref_<user_id>`;
6. получает личный кабинет и главное меню.

Реферальная ссылка:

```text
https://t.me/<bot_username>?start=ref_<user_id>
```

Администраторы проходят проверку каналов автоматически.

### Проверка обязательной подписки

Механизм находится в `services/context.py`.

- Проверяются все каналы из `required_channels`.
- Результат кэшируется на 60 секунд.
- Кнопка «Проверить подписку» сбрасывает кэш.
- Состояния `left` и `kicked` считаются неподпиской.
- Ошибка Telegram API также считается неподпиской.
- Для mirror-ботов gate можно отключать отдельно.
- При пустом списке обязательных каналов доступ разрешается всем.

### Главное меню

Пользователю доступны:

- «Кабинет» — открытие Telegram Mini App;
- «Мой баланс»;
- «Мой VPN»;
- «Купить VPN»;
- «Трафик»;
- бесплатный пробный период на 3 дня;
- «Рефералы»;
- «Зеркала бота»;
- «Поддержка»;
- «Информация»;
- выбор языка (русский/английский);
- ссылка на `KrepostStars_bot`;
- «Админ-панель» для администраторов.

---

## 3. Тарифы, баланс и покупки

### Баланс

Баланс пользователя хранится в `users.balance_rub` и используется для внутренних покупок. Поддерживаются:

- пополнение через платёжного провайдера;
- списание за VPN, подписку или трафик;
- начисление реферального вознаграждения;
- промокоды на баланс.

Списание выполняется атомарным условным SQL `UPDATE`, чтобы параллельные покупки не использовали одни и те же средства.

### Тарифы из текущего `config.yml`

VPN-тарифы:

| Код | Срок | Цена | Трафик |
|---|---:|---:|---:|
| `weekly` | 168 часов | 149 ₽ | 25 ГБ |
| `monthly` | 720 часов | 299 ₽ | 25 ГБ |
| `quarterly` | 2160 часов | 799 ₽ | 75 ГБ |
| `semiannual` | 4320 часов | 1599 ₽ | 150 ГБ |
| `yearly` | 8760 часов | 2799 ₽ | 300 ГБ |

Пакеты трафика:

| Код | Объём | Цена |
|---|---:|---:|
| `traffic_25gb` | 25 ГБ | 200 ₽ |
| `traffic_75gb` | 75 ГБ | 550 ₽ |
| `traffic_150gb` | 150 ГБ | 1000 ₽ |
| `traffic_300gb` | 300 ГБ | 1890 ₽ |

Комбинированный продукт:

- `traffic_subscription` — 30 дней и 25 ГБ за 300 ₽, `product: both`.

`weekly` присутствует в конфигурации, но намеренно скрыт в текущем Telegram UI. Покупка трафика разрешается только при активной подписке. Продление подписки требует остатка трафика. Проверки выполняются в `handlers/tariffs.py` через `check_purchase_allowed()`.

### Покупка с баланса

1. Пользователь выбирает тариф.
2. При необходимости вводит скидочный промокод.
3. Сумма атомарно списывается с баланса.
4. Создаётся invoice с provider ID `balance`.
5. Invoice сразу переводится в `paid`.
6. Запускается provisioning VPN.
7. При ошибке provisioning выполняются возврат средств и отмена invoice.
8. Флаг `fulfillment_applied` не допускает повторную выдачу.

---

## 4. Пробный доступ

Пробный тариф реализован в `handlers/tariffs.py`:

- срок — 72 часа;
- лимит — 10 ГБ;
- тарифный код invoice — `trial`;
- invoice сразу считается оплаченным;
- доступ создаётся через текущий VPN provider.

Trial недоступен, если пользователь когда-либо имел VPN-доступ или уже использовал пробный период. На provisioning устанавливается per-user `asyncio.Lock`.

При ошибке provisioning:

- удаляется созданный VPN key;
- сбрасывается `has_vpn_access`;
- invoice переводится в `cancelled`.

---

## 5. Промокоды

Таблицы: `promo_codes`, `promo_redemptions`. Реализация: `db/promo.py`.

Типы промокодов:

- `balance` — зачисляет рубли;
- `discount` — уменьшает цену тарифа.

Скидочный промокод может быть ограничен:

- конкретными тарифами;
- продуктом (`vpn`, `subscription`, `traffic`, `both`);
- максимальным количеством активаций;
- одним использованием на пользователя.

Операции доступны через Telegram, Mini App и Web Admin:

- проверка;
- активация;
- создание администратором;
- удаление администратором.

---

## 6. Платежи

### CryptoBot

Поддерживаются mainnet/testnet, создание и проверка invoice, transfer API и mock-режим. Testnet выбирается через `payments.use_testnet`.

### Platega

Поддерживаются создание invoice, polling статуса, webhook, пополнение баланса и прямая покупка тарифа.

### Anore

Поддерживаются создание и проверка платежа, optional HMAC request signature и webhook endpoint. В текущем обработчике webhook подпись фактически не проверяется до применения уведомления.

### Telegram Stars

Поддерживаются пополнение баланса и прямая покупка через `send_invoice`, `pre_checkout_query` и `successful_payment`. Повторная обработка защищена по `telegram_payment_charge_id`. Provider ID имеет вид `stars:<amount>`.

### Прямая покупка

Пользователь может оплатить тариф без пополнения внутреннего баланса. После подтверждения invoice помечается оплаченным, применяется промокод, запускается provisioning и отправляются уведомления пользователю и администраторам. `fulfillment_applied` защищает от повторной выдачи.

### Пополнение баланса

Invoice создаётся активным. После polling или webhook:

1. invoice становится `paid`;
2. `balance_applied` гарантирует однократное зачисление;
3. баланс увеличивается;
4. при необходимости начисляется реферальный бонус.

### Polling и webhooks

Если режим не `mock`, бот запускает polling активных invoice CryptoBot/Platega/Anore. Интервал по умолчанию — 15 секунд (`VPN_BOT_PAYMENT_POLL_INTERVAL_SECONDS`). Истёкшие invoice переводятся в `expired`.

Endpoints:

```text
POST /pay/notify
POST /api/pay/platega/notify
POST /api/pay/anore/notify
```

Payload логируется и может пересылаться администраторам. В текущей реализации webhook Platega и Anore не имеют полноценной криптографической проверки подписи.

### Распределение платежей

`distribution.py` содержит расчёт allocations, таблицу `payment_allocations` и CryptoBot transfer methods, но автоматический flow с `calculate_allocations()` в текущем runtime не подключён.

---

## 7. VPN provisioning

Реализация находится в `vpn.py`.

### Mock provider

Mock provider создаёт детерминированный access key и рассчитывает срок. Он предназначен для разработки: реальные renew/revoke/delete не поддерживаются и возвращают ошибки.

### Remnawave provider

Используются API:

```text
GET    /api/users/by-telegram-id/{telegram_id}
POST   /api/users
PATCH  /api/users
DELETE /api/users/{uuid}
GET    /api/internal-squads
```

При создании или обновлении пользователя:

- ищется существующий Remnawave user по Telegram ID;
- активный срок продлевается от текущего `expireAt`;
- рассчитывается traffic limit;
- передаются internal/external squads, лимит в байтах, стратегия лимита, Telegram ID, tag и description;
- сохраняется `subscriptionUrl`.

Если `internal_squad_uuids` пуст, выбирается squad `Default-Squad`.

### Срок и трафик

Продление выполняется через `update_expiration()`. Срок добавляется к существующему сроку; отрицательное изменение не позволяет установить дату раньше текущего времени более чем на минуту. Текущий traffic limit сохраняется.

Поддерживаются:

- добавление трафика;
- добавление трафика вместе с подпиской;
- абсолютная установка лимита;
- синхронизация лимита с Remnawave;
- чтение usage из Remnawave.

Текущая стратегия `trafficLimitStrategy` — `NO_RESET`.

### Отзыв и удаление

Администратор может отозвать или удалить VPN пользователя в Remnawave, удалить локальные ключи и полностью удалить локальную запись пользователя. Полное удаление необратимо на уровне приложения.

---

## 8. Subscription aggregator

Каталог находится в `src/vpn_sales_bot/aggregator/`.

### Объединение источников

Персональная ссылка имеет вид:

```text
/sub/<user_id>?token=<sha256-slug>
```

Агрегатор использует Remnawave `subscriptionUrl` и может объединять его с:

- внешними subscription sources;
- `static_vless`;
- legacy local access key;
- bypass pool.

Дубликаты узлов удаляются.

### Форматы клиентов

Распознаются Happ, v2rayNG, Streisand, Karing, Clash/Mihomo, Shadowrocket, sing-box, Hiddify, Nekobox/Nekoray, Loon и INCY.

Генерируются:

- Xray JSON;
- sing-box JSON;
- Clash/Mihomo YAML;
- generic base64.

Рендереры находятся в `aggregator/renderers/`.

### Автоматический выбор сервера

Для Xray:

- создаётся balancer;
- поддерживаются `leastLoad` и `leastPing`;
- выполняются health probes;
- обычные узлы приоритетнее узлов с remark `Обход`;
- добавляется direct routing для private/bittorrent и заданных доменов.

Для sing-box создаются `urltest` и selector с direct routing.

### Browser/HTTPS behavior

`/sub/{user_id}` различает браузер и VPN-клиент по User-Agent. Браузер получает React SPA при наличии build либо legacy HTML. VPN-клиент получает конфигурацию.

Поддерживаются специальные маршруты:

```text
/happ/{user_id}?token=...
/incy/{user_id}?token=...
```

Response headers могут содержать `Profile-Title`, `announce`, `Profile-Update-Interval`, `Subscription-Userinfo`, `Support-Url`, `Profile-Web-Page-Url`, `providerid`, `sub-info-color` и `Content-Disposition`.

Каждый запрос записывается в `subscription_request_logs`: клиент, формат, HWID, User-Agent, адрес, количество узлов и время.

---

## 9. React Telegram Mini App

Исходники: `frontend/src/`.

Маршруты:

```text
/                  landing
/site              site page
/sub               subscription landing
/sub/:userId       subscription page
/pay/success       успешная оплата
/pay/fail          неуспешная оплата
/app               dashboard
/app/buy           покупка
/app/invoice       invoice
/app/balance       баланс и пополнение
/app/settings      настройки
/app/guide         инструкция
```

Основные страницы: `AppDashboard`, `AppBuy`, `AppInvoice`, `AppBalance`, `AppSettings`, `AppGuide`, `Subscription`.

API Mini App:

```text
POST /api/app/auth
GET  /api/app/user/{user_id}
GET  /api/app/tariffs
GET  /api/app/payment-providers
POST /api/app/invoice/create/{user_id}
POST /api/app/trial/activate/{user_id}
POST /api/app/promo/validate/{user_id}
POST /api/app/promo/redeem/{user_id}
POST /api/app/balance/topup/{user_id}
GET  /api/app/admin-login-url/{user_id}
GET  /api/invoice/{invoice_id}/status
```

Авторизация строится на подписанном Telegram `window.Telegram.WebApp.initData`. Backend проверяет Telegram HMAC и выдаёт отдельный HMAC-based app token. Чувствительные endpoints требуют этот token.

Важно: Telegram handler ограничивает top-up значением `100000`, но Mini App endpoint в текущем коде не имеет такого же верхнего ограничения.

---

## 10. Реферальная система

Модули: `db/referrals.py`, `handlers/menu.py`, `bot.py`.

Сценарий:

1. пользователь входит с `ref_<id>`;
2. referrer записывается только один раз;
3. после успешного top-up начисляется процент;
4. уникальный `source_invoice_id` не позволяет начислить один бонус повторно;
5. referrer получает уведомление.

Текущие настройки:

```yaml
referral:
  enabled: true
  reward_percent: 10
```

---

## 11. Mirror-боты

Модули: `mirrors.py`, `handlers/mirrors.py`, `db/mirrors.py`.

Пользователь может добавить mirror-бота, передав токен BotFather. Бот:

1. проверяет токен через `get_me()`;
2. запрещает токен основного бота;
3. сохраняет зеркало в `bot_mirrors`;
4. запускает отдельный aiogram polling task.

Для зеркала доступны включение/выключение, удаление, настройка обязательной подписки, просмотр числа пользователей и активных VPN. Все зеркала используют общую БД, платежи, VPN provider и factory handlers.

---

## 12. Telegram Admin Panel

Доступ ограничен `bot.admin_ids`.

Команды:

```text
/admin
/balance USER_ID AMOUNT
```

Функции:

- dashboard и статистика;
- список клиентов с фильтрами active/inactive/all;
- поиск по ID/username;
- сортировка по регистрации или платежам;
- карточка клиента;
- add/subtract/set баланса;
- продление и сокращение подписки;
- revoke VPN;
- изменение traffic limit;
- просмотр и удаление платежей;
- создание и удаление промокодов;
- broadcast всем пользователям;
- ручной backup БД;
- синхронизация пользователей с Remnawave;
- миграция traffic limits;
- управление admin notifications;
- управление mirror-ботами;
- генерация одноразовой ссылки Web Admin.

Broadcast принимает любой Telegram content, ограничивает скорость примерно 25 сообщениями в секунду, обрабатывает `RetryAfter`, считает blocked/failed и удаляет исходное сообщение после рассылки.

Длительности в админском flow принимаются только в форматах `1d`, `7d`, `30d`, `12h`.

---

## 13. Web Admin Panel

Реализация: `admin_panel.py`, статические файлы — `admin_static/`. Включается через `admin_panel.enabled`.

Аутентификация и защита:

- Telegram Login Widget с HMAC validation;
- whitelist `bot.admin_ids`;
- signed expiring cookie;
- CSRF double-submit cookie/header;
- одноразовые login links;
- audit log privileged operations.

Основные маршруты:

```text
GET  /login
GET  /auth
GET  /login-link
GET  /logout
GET  /
GET  /m

GET  /api/summary
GET  /api/timeseries
GET  /api/audit
GET  /api/users
GET  /api/users/{user_id}

POST /api/users/{user_id}/balance
POST /api/users/{user_id}/traffic
POST /api/users/{user_id}/subscription
POST /api/users/{user_id}/{action}

GET  /api/servers
POST /api/server-actions/{action}
GET  /api/db-backup

GET    /api/payments
GET    /api/payments/{invoice_id}
DELETE /api/payments/{invoice_id}

GET    /api/promos
POST   /api/promos
DELETE /api/promos/{code}

GET  /api/mirrors
POST /api/mirrors/{mirror_id}/{action}

GET  /api/config
POST /api/config
```

Админские действия над пользователями: баланс, трафик, extend/reduce подписки, sync с Remnawave, revoke и delete. Сброс `reset_used` намеренно не поддерживается, потому что локальный reset не сбросил бы удалённое usage в Remnawave.

Web Admin также сохраняет Remnawave online snapshots примерно каждые 30 минут. Сохранение конфигурации создаёт backup вида `config.yml.bak-<unix_timestamp>`.

---

## 14. Фоновые процессы

### Polling платежей

По умолчанию каждые 15 секунд проверяются активные счета CryptoBot, Platega и Anore. Истёкшие счета получают статус `expired`, а неуспешные/истёкшие платежи уведомляют пользователя.

### Уведомления подписки

По умолчанию раз в час отправляются:

- предупреждение за 3 дня до окончания;
- одно уведомление после окончания.

Повторы блокируются таблицей `subscription_notifications`.

### Уведомления трафика

По умолчанию каждые 30 минут проверяются пороги 5, 3 и 1 ГБ. Usage читается из Remnawave. После покупки или добавления трафика уведомления сбрасываются.

Особенность: при остатке меньше 1 ГБ за один цикл могут последовательно отправиться уведомления всех трёх порогов.

### SQLite backups

По умолчанию каждый час выполняется SQLite Backup API и backup отправляется в Telegram chat `5874936084`. Настройки:

```text
VPN_BOT_DB_BACKUP_ENABLED
VPN_BOT_DB_BACKUP_CHAT_ID
VPN_BOT_DB_BACKUP_INTERVAL_SECONDS
```

### Daily report

Опциональный отчёт по времени Europe/Moscow содержит новых пользователей, revenue, top-ups, trials, активных VPN users и revenue по провайдерам.

---

## 15. База данных

SQLite включается в WAL mode и использует `busy_timeout`, `foreign_keys`, `synchronous=NORMAL` и транзакции `BEGIN IMMEDIATE`.

Основные таблицы:

- `users`;
- `invoices`;
- `vpn_keys`;
- `traffic_limits`;
- `traffic_notifications`;
- `subscription_notifications`;
- `subscription_request_logs`;
- `referral_rewards`;
- `promo_codes`;
- `promo_redemptions`;
- `stars_charges`;
- `payment_allocations`;
- `bypass_links`;
- `bypass_events`;
- `bot_mirrors`;
- `admin_audit_log`;
- `admin_login_tokens`;
- `remnawave_online_snapshots`.

Схема создаётся и частично мигрируется внутри `Database.init()`; отдельного migration framework нет.

Состояние runtime `bot.db` на момент исследования: 460 users, 211 invoices, 90 VPN keys, 4 mirrors, 4 promo codes и 154 bypass links.

---

## 16. Bypass pool

БД поддерживает `bypass_links` и `bypass_events`. Legacy-файл `bypass_pool.yml` используется как источник импорта при включённом соответствующем aggregator flow.

Текущие настройки указывают:

```yaml
aggregator:
  bypass_client:
    enabled: true
  bypass_pool_file: bypass_pool.yml
```

---

## 17. Конфигурация и запуск

Порядок загрузки:

1. `config.yml`;
2. поверх — `config.local.yml`, если существует;
3. отдельные runtime-параметры могут быть переопределены environment variables.

Поддерживаются переменные:

```text
VPN_BOT_ROOT
VPN_BOT_CONFIG
VPN_BOT_DB
VPN_BOT_LISTEN_HOST
VPN_BOT_LISTEN_PORT
VPN_BOT_ADMIN_LISTEN_HOST
VPN_BOT_ADMIN_LISTEN_PORT
VPN_BOT_DB_BACKUP_ENABLED
VPN_BOT_DB_BACKUP_CHAT_ID
VPN_BOT_DB_BACKUP_INTERVAL_SECONDS
VPN_BOT_SUBSCRIPTION_NOTIFY_ENABLED
VPN_BOT_SUBSCRIPTION_NOTIFY_INTERVAL_SECONDS
VPN_BOT_TRAFFIC_NOTIFY_ENABLED
VPN_BOT_TRAFFIC_NOTIFY_INTERVAL_SECONDS
VPN_BOT_PAYMENT_POLL_INTERVAL_SECONDS
```

В конфигурации обнаружены блоки `crystalpay` и `one_plat`, но текущий parser их не использует. Рабочие платёжные интеграции — Platega и Anore.

---

## 18. Deployment

Файлы deployment:

- `Dockerfile`;
- `docker-compose.example.yml`;
- `DEPLOYMENT.md`;
- `deploy/nginx/krepost-vpn.conf`;
- `.github/workflows/docker-image.yml`.

Docker build состоит из Node 20 frontend stage и Python 3.11 runtime. Запуск:

```text
/app/venv/bin/python -m vpn_sales_bot.bot
```

Runtime paths:

```text
/app/data/config.yml
/app/data/bot.db
```

Compose публикует локально `127.0.0.1:8088:8088` и `127.0.0.1:8090:8090`. Рекомендуемая схема — nginx с TLS перед aggregator/API/SPA, а Web Admin не публиковать напрямую наружу.

---

## 19. Важные ограничения и фактические риски

1. Platega и Anore webhook принимаются без полноценной проверки подписи.
2. Автоматический payout/distribution flow подготовлен, но не подключён.
3. Mini App top-up не повторяет лимит `100000`, установленный Telegram flow.
4. `config.yml` содержит live-looking секреты; такой файл следует считать скомпрометированным при утечке.
5. `weekly` настроен, но скрыт интерфейсом.
6. Mock provider не поддерживает renew/revoke/delete.
7. Часть UI читает usage из локальной БД, а часть — напрямую из Remnawave.
8. Пороговые traffic notifications могут отправиться несколькими сообщениями в одном цикле.
9. Legacy subscription slug предсказуем и подходит только для read-only subscription access; sensitive Mini App API использует отдельный token.
10. При `cookie_secure: true` Web Admin через обычный HTTP не сможет сохранить cookie.
11. Ошибки внешнего API в фоновых задачах могут влиять на общий event loop процесса.
12. Полное удаление пользователя удаляет локальные invoices, keys, referrals, promo redemptions и mirrors.

---

## 20. Тестовое покрытие

В `tests/unit/` и `tests/integration/` проверяются:

- Telegram authentication;
- статистика платежей;
- промокоды;
- subscription и entitlement services;
- Xray/sing-box/Clash renderers;
- преобразование node names;
- request classification;
- notifications;
- idempotency invoice/payment;
- удаление пользователя;
- admin users;
- VPN provider;
- DB key handling;
- INCY links.

Основные файлы зависимостей и настроек: `pyproject.toml`, `requirements.lock`, `requirements-dev.lock`, `pytest.ini`.

---

## 21. Карта пользовательских сценариев

```text
/start
  ├─ обязательные каналы
  ├─ реферальная привязка
  └─ главное меню
       ├─ Mini App / кабинет
       ├─ баланс ── пополнение ── webhook/polling ── referral reward
       ├─ купить VPN ── промокод ── balance/direct payment ── provisioning
       ├─ купить трафик ── проверка активной подписки ── sync Remnawave
       ├─ trial 3 дня ── одноразовая выдача
       ├─ мой VPN ── subscription URL / usage / продление
       ├─ рефералы ── ссылка и начисления
       ├─ зеркала ── проверка токена ── отдельный polling bot
       ├─ поддержка / информация
       └─ admin ── пользователи, платежи, VPN, broadcast, backup

/sub/{user_id}
  ├─ браузер ── React SPA или legacy HTML
  └─ VPN client ── Xray/sing-box/Clash/base64 config
```
