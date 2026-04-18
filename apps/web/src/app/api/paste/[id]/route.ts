import { NextResponse } from "next/server";
import { Agent, fetch as undiciFetch } from "undici";

const RUST_API_URL = process.env.RUST_API_URL || "http://localhost:8080";

export const runtime = "nodejs";
// Reads also need generous timeout: 3 attempts × ~60s worst-case + backoff.
export const maxDuration = 300;

// Same story as POST: bypass Node's 5-min undici headersTimeout ceiling.
const longPollAgent = new Agent({
  headersTimeout: 300_000,
  bodyTimeout: 300_000,
  connectTimeout: 10_000,
});

export async function GET(
  _req: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;

  try {
    const res = await undiciFetch(`${RUST_API_URL}/paste/${id}`, {
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