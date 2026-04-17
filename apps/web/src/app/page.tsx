"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";

export default function Home() {
  const [content, setContent] = useState("");
  const [loading, setLoading] = useState(false);
  const router = useRouter();

  async function handleSave() {
    if (!content.trim()) return;
    setLoading(true);
    try {
      const res = await fetch("/api/paste", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ content }),
      });
      if (!res.ok) throw new Error("Failed to save");
      const data = await res.json();
      if (data.id) router.push(`/p/${data.id}`);
    } catch (err) {
      console.error(err);
      alert("Something went wrong. Please try again.");
    } finally {
      setLoading(false);
    }
  }

  return (
    <main className="min-h-screen flex flex-col bg-background text-foreground">
      <header className="border-b border-border/40">
        <div className="mx-auto max-w-5xl px-6 py-4 flex items-center justify-between">
          <div className="flex items-center gap-2">
            <div className="size-7 rounded-md bg-gradient-to-br from-orange-400 to-pink-500" />
            <span className="font-semibold tracking-tight">Paste4Ever</span>
          </div>
          <a
            href="https://github.com/Jabs1989/paste4ever"
            target="_blank"
            rel="noreferrer"
            className="text-sm text-muted-foreground hover:text-foreground transition"
          >
            GitHub
          </a>
        </div>
      </header>

      <section className="mx-auto max-w-3xl w-full px-6 pt-16 pb-10 text-center">
        <h1 className="text-4xl sm:text-5xl font-bold tracking-tight mb-4">
          Paste anything. Keep it forever.
        </h1>
        <p className="text-lg text-muted-foreground mb-2">
          Permanent, decentralized pastebin. No accounts. No expiry.
        </p>
        <p className="text-sm text-muted-foreground">
          Powered by the{" "}
          <a
            href="https://autonomi.com"
            target="_blank"
            rel="noreferrer"
            className="underline hover:text-foreground"
          >
            Autonomi
          </a>{" "}
          network.
        </p>
      </section>

      <section className="mx-auto max-w-3xl w-full px-6 pb-24 flex-1">
        <Textarea
          placeholder="Paste your text, code, or notes here..."
          value={content}
          onChange={(e) => setContent(e.target.value)}
          className="min-h-[400px] font-mono text-sm resize-none"
          disabled={loading}
        />
        <div className="mt-4 flex items-center justify-between">
          <span className="text-xs text-muted-foreground">
            {content.length.toLocaleString()} characters
          </span>
          <Button
            onClick={handleSave}
            disabled={!content.trim() || loading}
            size="lg"
          >
            {loading ? "Saving..." : "Save Forever"}
          </Button>
        </div>
      </section>

      <footer className="border-t border-border/40">
        <div className="mx-auto max-w-5xl px-6 py-6 text-sm text-muted-foreground flex items-center justify-between">
          <span>Stored permanently on Autonomi</span>
          <span>© 2026 Paste4Ever</span>
        </div>
      </footer>
    </main>
  );
}