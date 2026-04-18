import { NextResponse } from "next/server";
import { Agent, fetch as undiciFetch } from "undici";

const RUST_API_URL = process.env.RUST_API_URL || "http://localhost:8080";

export const runtime = "nodejs";
// /health is a 3s antd probe at worst. Short timeout keeps a dead backend
// from blocking the homepage poll.
export const maxDuration = 15;
export const dynamic = "force-dynamic";

const agent = new Agent({
  headersTimeout: 5_000,
  bodyTimeout: 5_000,
  connectTimeout: 3_000,
});

export async function GET() {
  try {
    const res = await undiciFetch(`${RUST_API_URL}/health`, { dispatcher: agent });
    const data = await res.json();
    // No cache — the point of this endpoint is a fresh read of state.
    return NextResponse.json(data, {
      status: res.status,
      headers: { "Cache-Control": "no-store" },
    });
  } catch {
    // Rust API itself is down — that's also a degraded state from the user's POV.
    return NextResponse.json(
      { status: "degraded", antd_reachable: false, consecutive_failures: 0, api_reachable: false },
      { status: 200, headers: { "Cache-Control": "no-store" } },
    );
  }
}
