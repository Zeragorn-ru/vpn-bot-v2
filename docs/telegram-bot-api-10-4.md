# Telegram Bot API 10.4 и rich text contract

Этот контракт предназначен для Rust Telegram integration boundary. Он отделяет Telegram transport от доменной логики и не допускает разрозненных `reqwest` payloads в handlers/workers.

## Compatibility

- Decode updates with optional fields and ignore unknown fields for forward compatibility.
- Support message, callback query, pre-checkout query, successful payment, and webhook update IDs.
- Persist/claim `update_id` before business processing; duplicate delivery is acknowledged without duplicate side effects.
- Use webhook secret token validation, bounded request body, HTTPS at the reverse proxy, and fast acknowledgment.

## Rich text policy

Default `parse_mode` is `HTML`. Only `<b>`, `<strong>`, `<i>`, `<em>`, `<u>`, `<s>`, `<code>`, `<pre>`, `<blockquote>`, and safe `<a href="https://...">` are allowed. Dynamic values are escaped before insertion. Templates cannot accept raw markup from users, payment providers, or settings.

The renderer must reject unbalanced tags, unsupported attributes, unsafe URL schemes, text over 4096 characters, and captions over 1024 characters. A MarkdownV2 renderer is a separate opt-in mode and must escape the complete Telegram MarkdownV2 reserved-character set; formats must never be mixed.

## Typed operations

The client boundary exposes typed methods for `sendMessage`, `editMessageText`, `editMessageReplyMarkup`, `answerCallbackQuery`, media captions, and Stars `answerPreCheckoutQuery`. It maps Telegram responses into:

- transient errors (`429 RetryAfter`, network, 5xx) with bounded exponential retry;
- permanent errors (invalid chat/message/entity, forbidden) without retry;
- domain outcomes (already edited/deleted message) handled idempotently.

Never log bot tokens, access URLs, init data, invoice payloads, or complete user-generated message content. Redact sensitive fields in tracing spans.

## Required fixtures

1. escaped `&`, `<`, `>`, `"` and apostrophes;
2. attempted HTML/URL injection;
3. valid edit and missing-message edit;
4. malformed entity and oversized message;
5. 429 with `retry_after` and bounded retry;
6. duplicate update and duplicate successful Stars payment;
7. pre-checkout answer timeout/retry;
8. unknown Bot API update field.
