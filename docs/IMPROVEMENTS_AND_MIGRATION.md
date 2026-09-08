# Улучшения и план миграции legacy → Rust v2

## Назначение

`LEGACY_BOT_FUNCTIONALITY.md` описывает наблюдаемое поведение старого Python/aiogram-бота. Этот документ описывает целевое поведение и инженерные улучшения. Legacy SQLite, YAML, токены, пользователи, балансы, VPN-ссылки и секреты в v2 **не импортируются**.

## Что переносим

| Область | Legacy | Rust v2 | Приоритет |
|---|---|---|---|
| Telegram enrollment | `/start`, язык, referral, required channels | `telegram-bot` + `api`, проверка HMAC и replay TTL | P0 |
| Commerce | тарифы, wallet, trial, promo, referral | `domain` + PostgreSQL ledger + outbox | P0 |
| Payments | invoice, polling/webhook, Stars | `integrations` adapters + billing worker | P0/P1 |
| VPN | Remnawave create/extend/block/delete, usage | provider port + provisioning worker | P0 |
| Cabinet | subscription/access/instructions | Mini App, opaque revocable access token | P0 |
| Administration | users, tariffs, payments, audit, settings | API + Admin SPA + RBAC | P0 |
| Notifications | payment/provisioning/expiry/broadcast | notification outbox and worker | P0/P1 |
| Mirrors/bypass/legacy aggregators | shared bot tokens and external pools | security/legal review before separate release | P2 |

## Улучшения по сравнению с legacy

### Деньги и идемпотентность

- Деньги — целые `amount_minor`, без floating point.
- Баланс меняется только immutable wallet ledger transaction; invoice state и credit фиксируются одной PostgreSQL-транзакцией.
- Уникальны `(provider, provider_event_id)`, provider invoice ID и application idempotency key.
- Повторный webhook, Stars update или retry purchase возвращает исходный результат и не создаёт второй credit/entitlement.
- Provisioning использует persisted external idempotency key и состояние `provisioning_pending`, поэтому timeout безопасно повторяется.

### Безопасность

- Mini App identity принимается только из проверенного Telegram `initData`; client-supplied user ID игнорируется. Проверяются HMAC, `auth_date` TTL и Redis replay key.
- Access tokens — случайные opaque значения, revocable и не выводятся из Telegram ID; в PostgreSQL хранится только их hash. Публичные subscription URLs и пути — обычная конфигурация и не шифруются; raw access tokens не попадают в логи/audit diff.
- Платёжные callbacks принимаются только после provider-specific signature, timestamp, amount, currency и invoice binding checks.
- Secrets находятся в environment/secret storage, не в Git, SPA, audit и публичных настройках.
- Все admin mutations требуют RBAC, audit (actor/target/correlation/before-after), rate limit и защиту от duplicate submit.
- Удаление аккаунта анонимизирует PII, сохраняя обязательные финансовые и audit records.

### Rich text и Telegram Bot API 10.4

Единая Telegram integration boundary должна использовать Bot API 10.4-compatible DTO и forward-compatible decoding неизвестных update-полей. Бизнес-код не должен формировать raw `reqwest` payloads самостоятельно.

Правила сообщений:

1. Стандартный parse mode — `HTML`, разрешены только безопасные теги (`b`, `i`, `u`, `s`, `code`, `pre`, `a`, `blockquote`).
2. Любые пользовательские, payment-provider и admin-configured значения сначала проходят HTML escaping; они никогда не трактуются как markup.
3. Template renderer проверяет длину сообщения (4096), caption (1024), URL scheme (`https`/`tg://user` по месту) и не допускает незакрытые теги.
4. Typed client покрывает `sendMessage`, `editMessageText`, `editMessageReplyMarkup`, `answerCallbackQuery`, captions, `RetryAfter` и transient retry/backoff. Ошибки Telegram маппятся в permanent/transient классы.
5. `update_id` обрабатывается идемпотентно. Webhook ограничивает body size, проверяет secret header и отвечает быстро, передавая бизнес-событие в durable processing.
6. Stars `pre_checkout_query` и `successful_payment` остаются доверенным сигналом Telegram и используют общий transaction-safe fulfillment path.

Минимальный набор fixture-тестов: escaping `&<>"`, попытка инъекции `<a>`, edit успешного и отсутствующего сообщения, invalid entity, duplicate update, `RetryAfter`, oversized text и successful Stars replay.

### Надёжность и наблюдаемость

- PostgreSQL — source of truth; Redis используется только для ephemeral sessions, locks, rate limits, replay protection и cache.
- Transactional outbox с lease-aware `FOR UPDATE SKIP LOCKED`; worker retries имеют exponential backoff и DLQ/операторский видимый failure.
- `/healthz` проверяет liveness, `/readyz` — обязательные зависимости. Метрики/alerts: outbox backlog, webhook lag, payment failures, provisioning retries, notification failures, migration failures и unhealthy containers.
- Structured logs redacted by field, с correlation ID; access URLs, tokens и payment secrets запрещены в логах.

## CI/CD и deployment

### Pull request gates

1. `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
2. `npm ci` и `npm run build` для `web/admin` и `web/mini-app`.
3. `cargo audit`/dependency scan, secret scan и container scan.
4. SQL migrations применяются к disposable PostgreSQL на clean baseline и upgrade fixture; OpenAPI/Telegram fixtures проверяются в CI.

### Release

- Образы собираются только после gates, публикуются в GHCR по commit SHA.
- Production использует SHA digest/immutable SHA tag, не `latest`; образ сопровождается SBOM, provenance и vulnerability report.
- GitHub token получает минимальные permissions; production защищён environment approval и concurrency lock.
- На сервере нет source build: deploy только pull готовых образов, миграции запускаются до переключения сервисов.

### Безопасный rollout

1. Preflight проверяет `.env`, encryption key, Telegram token/webhook secret, свободное место и доступность backup destination.
2. Создаётся PostgreSQL backup; migration policy — forward-only, destructive migration только отдельным согласованным release.
3. Применяются упорядоченные migrations с `ON_ERROR_STOP`, затем pull SHA images и `docker compose up -d`.
4. Ожидаются health checks всех сервисов и API readiness; smoke проверяет webhook endpoint и ключевые dependencies.
5. Неуспешный rollout останавливает release и сохраняет diagnostics. Rollback меняет image SHA, но не удаляет PostgreSQL/Redis volumes; rollback schema выполняется только совместимым forward-fix.
6. SSH deploy выполняется отдельным unprivileged user с ограниченным ключом/командой; root login не нужен. Порты контейнеров слушают loopback, TLS остаётся на host reverse proxy.

## Acceptance matrix

- `/start` без/с валидным referral; referral нельзя заменить повторным входом.
- Required channel rejection и успешная повторная проверка.
- Trial — ровно один раз, 72 часа/10 GB по настройкам.
- Concurrent promo redemption и duplicate top-up reward.
- Signed webhook: invalid/stale/replayed/mismatched payloads rejected.
- Duplicate purchase idempotency key не создаёт второй invoice.
- Remnawave timeout → retry без второго external user.
- Rich-text escaping/edit/retry fixtures и Bot API 10.4 update compatibility.
- Clean install, backup/restore rehearsal, immutable deploy и tested rollback.

## Осознанно не переносим

Предсказуемые legacy subscription slugs, plaintext mirror credentials, unverified Platega/Anore callbacks, divergent validation между Telegram и Mini App, синхронный network I/O в business transaction, bypass pool и неиспользуемые payout allocation paths.
