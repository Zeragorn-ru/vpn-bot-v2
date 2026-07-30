# План разработки VPN Bot v2 (Целевое состояние)

## 1. Цель и ограничения

Создать публичный, поддерживаемый и переносимый продукт для продажи VPN-доступа в Telegram: Rust-бэкенд, современная админ-панель и Mini App, PostgreSQL + Redis, контейнерный деплой из одной папки на сервере.

### Обязательные принципы
- Функциональный паритет с текущим ботом для P0-сценариев, без переноса старых данных и конфигурации.
- v2 запускается как чистая инсталляция: пользователи, балансы, подписки, платежи и настройки из legacy не импортируются.
- Репозиторий не содержит токенов, паролей, дампов БД, пользовательских подписок или настоящих доменов.
- Конфигурация окружения хранит только инфраструктурные секреты и адреса. Все продуктовые и операционные параметры хранятся в PostgreSQL и редактируются в админке: бренд, тексты, тарифы, trial, промо/реферальные правила, каналы, статусы и доступность платёжных провайдеров, уведомления, расписания и client instructions.
- Каждый сервис stateless; постоянные данные размещаются только в PostgreSQL, Redis и явно смонтированных каталогах данных.
- На production-сервере не выполняется сборка исходников: CI публикует готовые immutable-образы в GHCR, сервер делает `pull` и перезапуск Compose-стека.
- Внешние API имеют версионированные контракты, проверку входных данных, структурированные ошибки и OpenAPI-описание.

## 2. Архитектура и Rust workspace

Cargo workspace с явным разделением приложений и общих библиотек:
- `apps/api`: REST API для админки, Mini App и Telegram Bot, биллинг, провижининг, уведомления.
- `apps/telegram-bot`: Telegram Bot (polling/webhook) — обработка команд и callback-ов.
- `apps/billing-worker`: обработка платежей и реконсиляция.
- `apps/provisioning-worker`: интеграция с Remnawave и outbox-воркеры.
- `apps/notification-worker`: доставка уведомлений в Telegram.
- `crates/domain`: доменные модели и бизнес-правила.
- `crates/storage`: PostgreSQL-репозитории и миграции.
- `crates/integrations`: Telegram, платёжные провайдеры (Crypto Pay, Anore, Telegram Stars), Remnawave.
- `crates/api-contracts`: DTO, OpenAPI-схемы, ошибки.
- `crates/observability`: tracing, метрики, health/readiness.
- `web/admin`: SPA админки.
- `web/mini-app`: Telegram Mini App.
- `deploy/`: docker-compose, миграции, скрипты.

## 3. Модули backend и хранение данных (PostgreSQL & Redis)

- **PostgreSQL**: Персистентное хранилище (пользователи, кошельки, инвойсы, подписки, тарифы, outbox, audit_log). Деньги хранятся в целых минимальных единицах валюты (`amount_minor`).
- **Transactional Outbox**: Асинхронная доставка событий через таблицу `outbox_events` с `FOR UPDATE SKIP LOCKED` и lease-aware воркером.
- **Redis**: Rate limiting (sliding window), кратковременные сессии, distributed locks и кэширование (cache-aside для тарифов и настроек).
- **Платёжные провайдеры**: Подключаемые адаптеры единого контракта с поддержкой webhook-ов, подписей и идемпотентности:
  - **Crypto Pay**: создание счетов в фиате (RUB) / крипте, проверка статуса, верификация HMAC-SHA256 вебхуков (`crypto-pay-api-signature`).
  - **Anore**: создание счетов, верификация подписи вебхука (`Anore-Signature`).
  - **Telegram Stars**: генерация invoice links через Bot API (`createInvoiceLink`), обработка pre-checkout и successful payment.
- **VPN-провижининг (Remnawave API v2)**: Создание, продление, блокировка и удаление клиентов через Bearer токен, синхронизация подписок и adaptive subscription gateway (выдача Xray JSON, sing-box JSON, Clash YAML и Base64 URI в зависимости от клиента).

## 4. Первичный деплой и эксплуатация

- Установка одной командой на чистый сервер через `setup.sh`.
- Базовый деплой через `docker-compose.yml` с локальной публикацией портов на `127.0.0.1` за reverse proxy (Nginx/Caddy) с TLS.
- Настройка параметров после первого входа через админ-панель (хранение в БД).
- Скрипты резервного копирования (`backup-postgres.sh`), обновления (`update.sh`) и отката (`rollback.sh`).
