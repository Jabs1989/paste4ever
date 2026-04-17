import { NextResponse } from "next/server";

const RUST_API_URL = process.env.RUST_API_URL || "http://localhost:8080";

export async function POST(req: Request) {
  const body = await req.json();

  try {
    const res = await fetch(`${RUST_API_URL}/paste`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
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