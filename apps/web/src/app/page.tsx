"use client";

import { useState, useEffect, useCallback } from "react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";

// Hot wallet that pays for every paste. Tips go directly into the same wallet,
// so a donation of ANT literally buys future pastes for other users. ETH also
// accepted (Arbitrum One gas). It's fine to expose — the private key lives on
// the server, not in the client.
const DONATION_ADDRESS = "0xA580e7f83C2DC7D59108cdB4c8716EBffA9A9B3C";
const ARBISCAN_URL = `https://arbiscan.io/address/${DONATION_ADDRESS}`;

type RecentPaste = {
  id: string;
  created_at: number;
  size_bytes: number;
  preview: string;
};

type Health = {
  status: "healthy" | "degraded";
  antd_reachable: boolean;
  consecutive_failures: number;
};

function timeAgo(unixSeconds: number): string {
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - unixSeconds);
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

export default function Home() {
  const [content, setContent] = useState("");
  const [loading, setLoading] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [recent, setRecent] = useState<RecentPaste[]>([]);
  const [copiedAddr, setCopiedAddr] = useState(false);
  const [health, setHealth] = useState<Health | null>(null);
  const router = useRouter();

  const loadHealth = useCallback(async () => {
    try {
      const res = await fetch("/api/health", { cache: "no-store" });
      if (!res.ok) return;
      const data = (await res.json()) as Health;
      setHealth(data);
    } catch {
      // If even the proxy blows up, pretend we don't know — badge hides itself.
    }
  }, []);

  const loadRecent = useCallback(async () => {
    try {
      const res = await fetch("/api/recent?limit=20", { cache: "no-store" });
      if (!res.ok) return;
      const data = (await res.json()) as RecentPaste[];
      if (Array.isArray(data)) setRecent(data);
    } catch {
      // Silent — wall is nice-to-have, not load-blocking.
    }
  }, []);

  useEffect(() => {
    loadRecent();
    // Refresh the wall every 30s so new pastes trickle in without a page reload.
    const t = setInterval(loadRecent, 30_000);
    return () => clearInterval(t);
  }, [loadRecent]);

  useEffect(() => {
    loadHealth();
    // Poll network health frequently enough that the badge reflects reality
    // within ~15s of a degradation — fast enough to warn before a user commits
    // to a doomed paste.
    const t = setInterval(loadHealth, 15_000);
    return () => clearInterval(t);
  }, [loadHealth]);

  // Tick an elapsed counter while uploading so the user knows we're not frozen.
  useEffect(() => {
    if (!loading) {
      setElapsed(0);
      return;
    }
    const start = Date.now();
    const interval = setInterval(() => {
      setElapsed(Math.floor((Date.now() - start) / 1000));
    }, 1000);
    return () => clearInterval(interval);
  }, [loading]);

  async function handleSave() {
    if (!content.trim()) return;
    setLoading(true);
    try {
      const res = await fetch("/api/paste", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ content }),
      });
      if (!res.ok) {
        const err = await res.json().catch(() => ({}));
        throw new Error(err.error || "Failed to save");
      }
      const data = await res.json();
      if (data.id) router.push(`/p/${data.id}`);
    } catch (err) {
      console.error(err);
      const msg =
        err instanceof Error
          ? err.message
          : "Something went wrong. Please try again.";
      alert(msg);
    } finally {
      setLoading(false);
    }
  }

  async function copyAddress() {
    await navigator.clipboard.writeText(DONATION_ADDRESS);
    setCopiedAddr(true);
    setTimeout(() => setCopiedAddr(false), 2000);
  }

  const mins = Math.floor(elapsed / 60);
  const secs = elapsed % 60;
  const timeStr = `${mins}:${String(secs).padStart(2, "0")}`;
  const shortAddr = `${DONATION_ADDRESS.slice(0, 6)}…${DONATION_ADDRESS.slice(-4)}`;

  return (
    <main className="min-h-screen flex flex-col bg-background text-foreground">
      <header className="border-b border-border/40">
        <div className="mx-auto max-w-5xl px-6 py-4 flex items-center justify-between">
          <div className="flex items-center gap-2">
            <div className="size-7 rounded-md bg-gradient-to-br from-orange-400 to-pink-500" />
            <span className="font-semibold tracking-tight">Paste4Ever</span>
          </div>
          <div className="flex items-center gap-4">
            {health && (
              <span
                className="flex items-center gap-1.5 text-xs text-muted-foreground"
                title={
                  health.status === "healthy"
                    ? "Autonomi network is healthy"
                    : `Network is degraded — ${health.consecutive_failures} recent failure(s). Saves may not land.`
                }
              >
                <span
                  className={`inline-block size-2 rounded-full ${
                    health.status === "healthy"
                      ? "bg-emerald-400"
                      : "bg-amber-400 animate-pulse"
                  }`}
                />
                {health.status === "healthy" ? "Network healthy" : "Reconnecting"}
              </span>
            )}
            <a
              href="https://github.com/Jabs1989/paste4ever"
              target="_blank"
              rel="noreferrer"
              className="text-sm text-muted-foreground hover:text-foreground transition"
            >
              GitHub
            </a>
          </div>
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

      <section className="mx-auto max-w-3xl w-full px-6 pb-16">
        {health?.status === "degraded" && !loading && (
          <div className="mb-4 rounded-lg border border-amber-400/40 bg-amber-400/10 p-3 text-sm text-amber-200">
            <span className="font-medium">Network is reconnecting.</span>{" "}
            Recent pastes have failed to land — we&apos;re restarting the upstream
            node. Try again in a minute or two. You won&apos;t be charged for
            failed uploads that never reach the network.
          </div>
        )}
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
            {loading ? `Storing on Autonomi... ${timeStr}` : "Save Forever"}
          </Button>
        </div>
        {loading && (
          <div className="mt-6 rounded-lg border border-border/40 bg-card/50 p-4 text-sm text-muted-foreground">
            <p className="mb-2">
              <span className="inline-block size-2 rounded-full bg-orange-400 animate-pulse mr-2" />
              Paying for permanent storage on Autonomi — this typically takes{" "}
              <span className="font-medium text-foreground">2 to 4 minutes</span>.
            </p>
            <p className="text-xs">
              Your paste is being split into encrypted chunks, paid for on
              Arbitrum, and distributed across the network. Please don&apos;t
              close this tab.
            </p>
          </div>
        )}
      </section>

      {/* ── Donation strip ────────────────────────────────────────────── */}
      <section className="mx-auto max-w-3xl w-full px-6 pb-12">
        <div className="rounded-xl border border-border/40 bg-card/30 p-5 text-sm">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <p className="font-medium text-foreground mb-1">
                Paste4Ever is free for everyone.
              </p>
              <p className="text-muted-foreground">
                Every paste costs a few cents of ANT on Arbitrum. Want to help
                keep it online? Send ANT or ETH to our storage wallet — it
                literally pays for the next stranger&apos;s paste.
              </p>
            </div>
            <div className="flex items-center gap-2 shrink-0">
              <code className="text-xs font-mono px-2 py-1 rounded bg-muted/50 border border-border/40">
                {shortAddr}
              </code>
              <Button variant="outline" size="sm" onClick={copyAddress}>
                {copiedAddr ? "Copied!" : "Copy"}
              </Button>
              <a
                href={ARBISCAN_URL}
                target="_blank"
                rel="noreferrer"
              >
                <Button variant="outline" size="sm">
                  Arbiscan
                </Button>
              </a>
            </div>
          </div>
        </div>
      </section>

      {/* ── Wall of pastes ────────────────────────────────────────────── */}
      <section className="mx-auto max-w-5xl w-full px-6 pb-24">
        <div className="flex items-baseline justify-between mb-6">
          <h2 className="text-2xl font-semibold tracking-tight">
            Recent pastes
          </h2>
          <span className="text-xs text-muted-foreground">
            {recent.length > 0
              ? `${recent.length} most recent`
              : "Be the first to paste"}
          </span>
        </div>

        {recent.length === 0 ? (
          <div className="rounded-lg border border-dashed border-border/50 py-16 text-center text-sm text-muted-foreground">
            No pastes yet. Yours could be the first one here forever.
          </div>
        ) : (
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
            {recent.map((p) => (
              <Link
                key={p.id}
                href={`/p/${p.id}`}
                className="group rounded-lg border border-border/40 bg-card/30 p-4 hover:bg-card/60 hover:border-border transition"
              >
                <p className="font-mono text-sm text-foreground/90 line-clamp-4 min-h-[4.5rem] break-words">
                  {p.preview || "(empty)"}
                </p>
                <div className="mt-3 flex items-center justify-between text-xs text-muted-foreground">
                  <span className="font-mono truncate" title={p.id}>
                    {p.id.slice(0, 10)}…
                  </span>
                  <span className="shrink-0 ml-2">
                    {timeAgo(p.created_at)} · {formatBytes(p.size_bytes)}
                  </span>
                </div>
              </Link>
            ))}
          </div>
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
