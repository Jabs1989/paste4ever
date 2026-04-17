import { NextResponse } from "next/server";
import { savePaste } from "@/lib/store";

export async function POST(req: Request) {
  const { content } = await req.json();

  if (!content || typeof content !== "string") {
    return NextResponse.json({ error: "Content required" }, { status: 400 });
  }
  if (content.length > 100_000) {
    return NextResponse.json(
      { error: "Content too large (max 100KB)" },
      { status: 413 }
    );
  }

  const id = savePaste(content);
  return NextResponse.json({ id });
}