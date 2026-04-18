# Paste4Ever

**Paste anything. Keep it forever.** A permanent, decentralized pastebin built on the [Autonomi](https://autonomi.com) network.

No accounts. No servers holding your data. No expiry. You paste, pay a few cents on Arbitrum, and the bytes live on the network forever — readable by anyone with the link.

---

## Status

🟢 **Live on Autonomi mainnet.** Real pastes stored, paid for on Arbitrum One, and readable end-to-end through the web UI.

Verifiable addresses from the current build:

| Address | Content |
|---|---|
| `095f68af9ccfd43e7f9c18699e302a0e74bc4765eb4e59c6d8b9b9a7df19b878` | `Hello Autonomi from Paste4Ever!` |
| `9c0e17646dbff961c616d2d0ae6527d4ff69b3a38d7a0363567fb9554106ab52` | API retry-logic regression test |
| `6be1ef727c5736b8397c57ffbaf273316e7929284862cd82fab124af7004bcc7` | First end-to-end paste created via the browser |

Anyone running `antd` against the Autonomi default network can `GET /v1/data/public/<address>` and read them back.

---

## Why it matters

Autonomi is a decentralized storage network where data is paid-once, stored forever, and addressed by content. It's a natural fit for pastebins: existing services like Pastebin or GitHub Gist depend on a company staying in business for your link to keep working. Paste4Ever removes that dependency — once a paste is stored, no company can delete it and no subscription can expire.

This repo is the first consumer-facing application of that idea on Autonomi mainnet.

---

## Architecture

```
┌─────────────┐    HTTP     ┌──────────────┐    HTTP     ┌──────┐    QUIC    ┌──────────────┐
│  Next.js 16 │  ─────────▶ │ paste4ever-  │ ─────────▶ │ antd │ ────────▶ │   Autonomi   │
│   (web)     │             │     api      │            │      │           │    network   │
│ App Router  │ ◀───────── │  (Rust/Axum) │ ◀───────── │      │ ◀──────── │   (+ EVM on  │
│ React 19    │             │              │            │      │           │  Arbitrum 1) │
└─────────────┘             └──────────────┘            └──────┘           └──────────────┘
```

- **`apps/web`** — Next.js 16 frontend. shadcn/ui, Tailwind, React 19. Two API routes proxy to the Rust service with a custom `undici` dispatcher (Node's default 5-min headers timeout is too short for Autonomi's first-seen pastes).
- **`apps/api`** — Rust/Axum gateway. Thin service that enforces paste-level business rules (size limits, future rate limits) and implements resilient upload/download with retry + exponential backoff. All P2P and payment logic lives in `antd`.
- **`antd`** — Autonomi network daemon (from [WithAutonomi/ant-sdk](https://github.com/WithAutonomi)). Talks to the Autonomi DHT and signs EVM payments on Arbitrum One.

## Stack

- **Frontend:** Next.js 16, React 19, TypeScript, Tailwind v4, shadcn/ui
- **Backend:** Rust, Axum, reqwest, tokio
- **Network:** Autonomi mainnet (saorsa-transport over QUIC)
- **Payments:** Arbitrum One (ANT for storage, ETH for gas)

## Repo layout

```
paste4ever/
├── apps/
│   ├── web/   # Next.js frontend
│   └── api/   # Rust/Axum gateway
└── README.md
```

---

## Running locally

Running the full stack requires a funded Arbitrum One wallet (ANT + a little ETH) and a local `antd` daemon talking to the Autonomi mainnet.

### 1. Start `antd`

Follow [antd's setup guide](https://github.com/WithAutonomi). Launch it with:

```powershell
$env:AUTONOMI_WALLET_KEY="0x..."
$env:EVM_RPC_URL="https://arbitrum-one.publicnode.com"
$env:EVM_NETWORK="arbitrum-one"
$env:EVM_PAYMENT_TOKEN_ADDRESS="0xa78d8321B20c4Ef90eCd72f2588AA985A4BDb684"
$env:EVM_PAYMENT_VAULT_ADDRESS="0x9A3EcAc693b699Fc0B2B6A50B5549e50c2320A26"
$env:ANTD_PEERS="/ip4/207.148.94.42/udp/10000/quic,..."
.\target\release\antd
```

### 2. Start the Rust API

```bash
cd apps/api
cargo run --release
# Listens on :8080, proxies to antd on :8082
```

### 3. Start the web app

```bash
cd apps/web
npm install
npm run dev
# http://localhost:3000
```

---

## Roadmap

- ✅ Working write/read through the full stack on Autonomi mainnet
- ✅ Resilient upload retry (chunk-storage flaps are common on the early-days network)
- 🟡 Rate limiting + Cloudflare Turnstile
- 🟡 Deploy frontend to Cloudflare Pages, API to Fly.io
- 🟡 Register `paste4ever` on AntNS
- 🟡 Syntax highlighting, expiry-free paste discovery, sharable short links

## License

MIT — see [LICENSE](./LICENSE).

---

Built by [@Jabs1989](https://github.com/Jabs1989). Powered by the [Autonomi](https://autonomi.com) network.
