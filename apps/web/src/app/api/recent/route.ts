import { NextResponse } from "next/server";
import { Agent, fetch as undiciFetch } from "undici";

const RUST_API_URL = process.env.RUST_API_URL || "http://localhost:8080";

export const runtime = "nodejs";
// /recent is a cheap SQLite read — short timeout is fine.
export const maxDuration = 30;
// Opt out of Next's route-level static caching so new pastes appear immediately.
export const dynamic = "force-dynamic";

const agent = new Agent({
  headersTimeout: 10_000,
  bodyTimeout: 10_000,
  connectTimeout: 5_000,
});

export async function GET(req: Request) {
  const url = new URL(req.url);
  const limit = url.searchParams.get("limit") ?? "20";

  try {
    const res = await undiciFetch(`${RUST_API_URL}/recent?limit=${encodeURIComponent(limit)}`, {
      dispatcher: agent,
    });
    const data = await res.json();
    // Cache the wall briefly on the edge. Pastes take minutes to upload so
    // we don't need sub-second freshness, and a 10s edge cache absorbs
    // stampedes if the homepage gets hammered.
    return NextResponse.json(data, {
      status: res.status,
      headers: { "Cache-Control": "public, s-maxage=10, stale-while-revalidate=60" },
    });
  } catch (err) {
    console.error("Rust API unreachable (recent):", err);
    return NextResponse.json([], { status: 200 });
  }
}
