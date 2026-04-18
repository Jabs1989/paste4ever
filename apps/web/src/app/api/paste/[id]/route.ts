import { NextResponse } from "next/server";

const RUST_API_URL = process.env.RUST_API_URL || "http://localhost:8080";

// Reads can take a while too: 3 attempts × ~60s worst-case + backoff on
// the Rust side. Native fetch for Workers runtime compatibility.
export const dynamic = "force-dynamic";
export const maxDuration = 300;

export async function GET(
  _req: Request,
  { params }: { params: Promise<{ id: string }> },
) {
  const { id } = await params;

  try {
    const res = await fetch(`${RUST_API_URL}/paste/${id}`, {
      signal: AbortSignal.timeout(300_000),
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
