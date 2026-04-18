import { NextResponse } from "next/server";

const RUST_API_URL = process.env.RUST_API_URL || "http://localhost:8080";

// Runs on the Cloudflare Workers runtime (via OpenNext) in production and on
// Node locally. We use native fetch — undici's custom Agent doesn't work on
// the Workers runtime, and the Rust API's /health is fast enough that we
// don't need a custom dispatcher anyway.
export const dynamic = "force-dynamic";

export async function GET() {
  try {
    const res = await fetch(`${RUST_API_URL}/health`, {
      signal: AbortSignal.timeout(5_000),
      // Opt out of any incidental caching between us and the origin.
      cache: "no-store",
    });
    const data = await res.json();
    return NextResponse.json(data, {
      status: res.status,
      headers: { "Cache-Control": "no-store" },
    });
  } catch {
    // API itself unreachable — surface that as a degraded state so the
    // homepage badge still has something to render against.
    return NextResponse.json(
      {
        status: "degraded",
        antd_reachable: false,
        consecutive_failures: 0,
        api_reachable: false,
      },
      { status: 200, headers: { "Cache-Control": "no-store" } },
    );
  }
}
