import { NextResponse } from "next/server";
import { Agent, fetch as undiciFetch } from "undici";

const RUST_API_URL = process.env.RUST_API_URL || "http://localhost:8080";

// Force Node runtime (not edge) so long-running fetch works.
export const runtime = "nodejs";
// Allow up to 15 min — Autonomi uploads can take 2-4 min per attempt, and the
// Rust API retries up to 3 times on congestion.
export const maxDuration = 900;

// Node's global fetch (undici) has a hardcoded 5-minute headersTimeout that
// AbortSignal.timeout() does NOT override. Our paste4ever-api can take longer
// (3 attempts × ~3min + backoff). We need a custom dispatcher with a bigger
// headers timeout, otherwise Node silently kills the socket at exactly 5:00
// even when the Rust backend is still happily working.
const longPollAgent = new Agent({
  headersTimeout: 900_000, // 15 min
  bodyTimeout: 900_000,
  connectTimeout: 10_000,
});

export async function POST(req: Request) {
  const body = await req.json();

  try {
    const res = await undiciFetch(`${RUST_API_URL}/paste`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
      dispatcher: longPollAgent,
    });

    const data = await res.json();
    return NextResponse.json(data, { status: res.status });
  } catch (err) {
    console.error("Rust API unreachable:", err);
    return NextResponse.json(
      { error: "Storage service unavailable" },
      { status: 503 }
    );
  }
}