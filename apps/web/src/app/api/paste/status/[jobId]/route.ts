import { NextResponse } from "next/server";

const RUST_API_URL = process.env.RUST_API_URL || "http://localhost:8080";

// Dev-only proxy for polling paste upload status. In production the browser
// calls api.paste4ever.com/paste/status/<id> directly (NEXT_PUBLIC_API_URL
// is set), but for local dev (no CORS, no tunnel) we forward through the
// Next.js server. Short timeout — each poll is a cheap in-memory lookup.
export const dynamic = "force-dynamic";

export async function GET(
  _req: Request,
  { params }: { params: Promise<{ jobId: string }> },
) {
  const { jobId } = await params;

  try {
    const res = await fetch(
      `${RUST_API_URL}/paste/status/${encodeURIComponent(jobId)}`,
      { signal: AbortSignal.timeout(5_000), cache: "no-store" },
    );
    const data = await res.json();
    return NextResponse.json(data, {
      status: res.status,
      headers: { "Cache-Control": "no-store" },
    });
  } catch (err) {
    console.error("Rust API unreachable (status):", err);
    return NextResponse.json(
      { error: "Storage service unavailable" },
      { status: 503 },
    );
  }
}
