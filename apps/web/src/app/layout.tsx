import type { Metadata } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import "./globals.css";

const geistSans = Geist({
  variable: "--font-geist-sans",
  subsets: ["latin"],
});

const geistMono = Geist_Mono({
  variable: "--font-geist-mono",
  subsets: ["latin"],
});

export const metadata: Metadata = {
  title: "Paste4Ever — Paste anything. Keep it forever.",
  description:
    "Permanent, decentralized pastebin powered by the Autonomi network. No accounts, no expiry, no gatekeeper.",
  openGraph: {
    title: "Paste4Ever",
    description: "Paste anything. Keep it forever. Powered by Autonomi.",
    url: "https://paste4ever.com",
    siteName: "Paste4Ever",
    type: "website",
  },
  twitter: {
    card: "summary_large_image",
    title: "Paste4Ever",
    description: "Paste anything. Keep it forever. Powered by Autonomi.",
  },
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en" className="dark">
      <body
        className={`${geistSans.variable} ${geistMono.variable} antialiased`}
      >
        {children}
      </body>
    </html>
  );
}