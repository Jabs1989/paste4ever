"use client";

import { useState, useEffect, use } from "react";
import Link from "next/link";
import { Button } from "@/components/ui/button";

type Params = Promise<{ id: string }>;

export default function PastePage({ params }: { params: Params }) {
  const { id } = use(params);
  const [content, setContent] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [linkCopied, setLinkCopied] = useState(false);

  useEffect(() => {
    fetch(`/api/paste/${id}`)
      .then((res) => res.json())
      .then((data) => {
        if (data.content !== undefined) setContent(data.content);
        else setError("Paste not found");
      })
      .catch(() => setError("Failed to load paste"))
      .finally(() => setLoading(false));
  }, [id]);

  async function copyContent() {
    if (!content) return;
    await navigator.clipboard.writeText(content);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }

  async function copyLink() {
    await navigator.clipboard.writeText(window.location.href);
    setLinkCopied(true);
    setTimeout(() => setLinkCopied(false), 2000);
  }

  return (
    <main className="min-h-screen flex flex-col bg-background text-foreground">
      <header className="border-b border-border/40">
        <div className="mx-auto max-w-5xl px-6 py-4 flex items-center justify-between">
          <Link href="/" className="flex items-center gap-2">
            <div className="size-7 rounded-md bg-gradient-to-br from-orange-400 to-pink-500" />
            <span className="font-semibold tracking-tight">Paste4Ever</span>
          </Link>
          <Link href="/">
            <Button variant="outline" size="sm">
              New Paste
            </Button>
          </Link>
        </div>
      </header>

      <section className="mx-auto max-w-3xl w-full px-6 py-10 flex-1">
        {loading && (
          <div className="text-center text-muted-foreground py-20">
            Loading paste...
          </div>
        )}

        {error && (
          <div className="text-center py-20">
            <p className="text-destructive mb-4">{error}</p>
            <Link href="/">
              <Button>Create a new paste</Button>
            </Link>
          </div>
        )}

        {content !== null && (
          <>
            <div className="flex items-center justify-between mb-4">
              <p className="text-xs text-muted-foreground font-mono">
                ID: {id}
              </p>
              <div className="flex gap-2">
                <Button variant="outline" size="sm" onClick={copyContent}>
                  {copied ? "Copied!" : "Copy"}
                </Button>
                <Button variant="outline" size="sm" onClick={copyLink}>
                  {linkCopied ? "Link copied!" : "Share"}
                </Button>
              </div>
            </div>

            <pre className="rounded-lg border border-border/40 bg-card p-4 overflow-auto font-mono text-sm whitespace-pre-wrap break-words min-h-[300px]">
              {content}
            </pre>

            <p className="mt-6 text-xs text-muted-foreground text-center">
              Stored permanently on the Autonomi network
            </p>
          </>
        )}
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