import type { NextConfig } from "next";
// Hook that lets `next dev` read Cloudflare env/bindings locally via
// miniflare. Harmless no-op when the adapter isn't needed, so we just
// always call it at module load.
import { initOpenNextCloudflareForDev } from "@opennextjs/cloudflare";

initOpenNextCloudflareForDev();

const nextConfig: NextConfig = {
  /* config options here */
};

export default nextConfig;
