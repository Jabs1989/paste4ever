// Temporary in-memory store for Wave 2.
// Wave 3 replaces this with real Autonomi uploads.

const pastes = new Map<string, string>();

export function savePaste(content: string): string {
  const id = Math.random().toString(36).substring(2, 10);
  pastes.set(id, content);
  return id;
}

export function getPaste(id: string): string | undefined {
  return pastes.get(id);
}