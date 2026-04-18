import { NextResponse } from "next/server";

const RUST_API_URL = process.env.RUST_API_URL || "http://localhost:8080";

// Wall-of-pastes read. Fast SQLite query on the Rust side, so native fetch
// with a short timeout is plenty. Works on the Workers runtime too.
export const dynamic = "force-dynamic";

export async function GET(req: Request) {
  const url = new URL(req.url);
  const limit = url.searchParams.get("limit") ?? "20";

  try {
    const res = await fetch(
      `${RUST_API_URL}/recent?limit=${encodeURIComponent(limit)}`,
      { signal: AbortSignal.timeout(10_000), cache: "no-store" },
    );
    const data = await res.json();
    // Cache briefly at the edge so the homepage doesn't hammer the tunnel
    // when multiple visitors load at once. 10s is short enough that new
    // pastes appear essentially immediately on the wall.
    return NextResponse.json(data, {
      status: res.status,
      headers: {
        "Cache-Control": "public, s-maxage=10, stale-while-revalidate=60",
      },
    });
  } catch (err) {
    console.error("Rust API unreachable (recent):", err);
    return NextResponse.json([], { status: 200 });
  }
}
