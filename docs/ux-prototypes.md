# UX Prototypes

These task-focused wireframes define the information architecture before visual
implementation. The Mini App is mobile-first; the administration SPA is
desktop-dense and responsive down to 320 px.

## Mini App

### Home

```text
+----------------------------------+
| Brand                    [RU/EN] |
|                                  |
| Your VPN                         |
| Active until 14 Aug              |
| 18 GB remaining                  |
|                                  |
| [ Open access ]                  |
|                                  |
| [ Renew ]       [ Choose tariff ]|
|                                  |
| Wallet: 299.00 RUB     [ Top up ]|
+----------------------------------+
```

### Catalog and Checkout

```text
+----------------------------------+
| < Tariffs                        |
| Monthly                           |
| 30 days · 25 GB                  |
| 299.00 RUB                       |
|                                  |
| Promo code                 [Add] |
| Wallet                    299 RUB|
| Total                     299 RUB|
|                                  |
| [ Pay with wallet ]              |
| [ Select payment method ]        |
+----------------------------------+
```

### Access

```text
+----------------------------------+
| < Access                         |
| Connection link                  |
| ************              [ Copy ]|
|                                  |
| 1. Install a supported client     |
| 2. Copy and import the link       |
| 3. Connect                         |
|                                  |
| [ View client instructions ]      |
+----------------------------------+
```

## Admin SPA

Visual direction: terminal-first operational UI inspired by Signal Dashboard's
data density and scanning rhythm: dark surfaces, compact monospaced controls,
green status signals, clear tables and charts. It does not reuse Signal
Dashboard assets, source code, branding, or copy.

### Dashboard

```text
+-----------+----------------------------------------------+
 | Overview  | Revenue       Payments       Active access     |
 | Clients   | Invoice pulse and operator action queue        |
 | Payments  |                                              |
 | Subs      | Recent clients / access / payment activity     |
 | Tariffs   +----------------------------------------------+
 | Promos    |                                               |
 | Channels  |                                               |
 | Providers |                                               |
 | Policies  |                                               |
+-----------+----------------------------------------------+
```

### Operations List

```text
+-----------------------------------------------------------+
| Payments     [period] [status] [provider] [search]        |
| ID        User       Amount      Provider    Status        |
| inv_...   @user      299 RUB     Crypto Pay  paid          |
| inv_...   @user      799 RUB     Crypto Pay  pending       |
|                                                           |
| < 1 2 3 >                         server pagination       |
+-----------------------------------------------------------+
```

## Common Acceptance Criteria

- Every screen has loading, empty, error, and retry states.
- Payment and provisioning outcomes are explicit; no action relies only on a
  spinner or optimistic success.
- Sensitive access links are obscured until an authenticated user requests a
  copy/open action.
- Telegram light/dark themes, safe areas, keyboard navigation, focus states,
  and contrast are verified before P0 release.
