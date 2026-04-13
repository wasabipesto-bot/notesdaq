const API_BASE = import.meta.env.PUBLIC_API_BASE || "/api/v1";

export async function api<T>(path: string): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`);
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    throw new Error(body.error || `API error: ${res.status}`);
  }
  return res.json();
}

export function formatNumber(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 10_000) return (n / 1_000).toFixed(1) + "k";
  return n.toLocaleString();
}

export function formatMultiplier(m: number): string {
  if (!isFinite(m)) return "+Inf";
  if (m >= 100) return m.toFixed(0) + "x";
  if (m >= 10) return m.toFixed(1) + "x";
  return m.toFixed(2) + "x";
}

export function relativeTime(dateStr: string): string {
  const diff = Date.now() - new Date(dateStr).getTime();
  const minutes = Math.floor(diff / 60000);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;
  const months = Math.floor(days / 30);
  if (months < 12) return `${months}mo ago`;
  return `${Math.floor(months / 12)}y ago`;
}

export function multiplierClass(m: number): string {
  if (m >= 2) return "gain";
  if (m < 1) return "loss";
  return "";
}
