import { NextResponse } from "next/server";

const RUST_API_URL = process.env.RUST_API_URL || "http://localhost:8080";

// Paste uploads take 2-4 minutes on Autonomi. On the Cloudflare Workers
// runtime, a single request can stay open for a long time as long as we
// keep awaiting a subrequest — the limit is CPU time, not wall time.
// Native fetch (not undici) so this works on Workers.
export const dynamic = "force-dynamic";
// Generous cap: 3 attempts × ~3min + backoff on the Rust side.
export const maxDuration = 900;

export async function POST(req: Request) {
  const body = await req.json();

  try {
    const res = await fetch(`${RUST_API_URL}/paste`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
      // 15 min ceiling. The Rust API has its own shorter per-attempt
      // timeouts; this is just the outer rope.
      signal: AbortSignal.timeout(900_000),
    });
    const data = await res.json();
    return NextResponse.json(data, { status: res.status });
  } catch (err) {
    console.error("Rust API unreachable:", err);
    return NextResponse.json(
      { error: "Storage service unavailable" },
      { status: 503 },
    );
  }
}
