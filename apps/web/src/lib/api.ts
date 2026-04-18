// Resolve the right base URL for a Paste4Ever API call.
//
// Two modes:
//   1. Production build: NEXT_PUBLIC_API_URL is set (e.g. https://api.paste4ever.com).
//      Calls go direct from the browser to the Rust API. This matters because:
//        a) The Worker hosting the frontend cannot reach api.paste4ever.com
//           via the tunnel — same-zone fetches return Cloudflare error 1003.
//        b) Skipping the Worker hop removes a ~30s request-duration ceiling
//           and saves Worker CPU time.
//        c) The Rust API already sets `Access-Control-Allow-Origin: *` so
//           cross-origin calls work.
//
//   2. Local dev: NEXT_PUBLIC_API_URL is empty/unset. Calls go to the
//      Next.js /api/* proxy routes which forward to http://localhost:8080.
//      This lets `next dev` work without a tunnel.
//
// Usage:
//   fetch(apiUrl("/health"))          // prod: https://api.paste4ever.com/health
//                                     // dev:  /api/health
//   fetch(apiUrl("/paste/abc123"))    // prod: https://api.paste4ever.com/paste/abc123
//                                     // dev:  /api/paste/abc123
const PUBLIC_API = process.env.NEXT_PUBLIC_API_URL?.replace(/\/$/, "");

export function apiUrl(path: string): string {
  if (PUBLIC_API) return `${PUBLIC_API}${path}`;
  return `/api${path}`;
}
