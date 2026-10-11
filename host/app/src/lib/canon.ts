/** A stable string for comparing values: sorted keys; null, undefined and [] dropped. */
export function canon(v: unknown): string {
  const strip = (x: unknown): unknown => {
    if (Array.isArray(x)) return x.map(strip);
    if (x && typeof x === "object") {
      const out: Record<string, unknown> = {};
      for (const k of Object.keys(x).sort()) {
        const val = (x as Record<string, unknown>)[k];
        if (val == null || (Array.isArray(val) && val.length === 0)) continue;
        out[k] = strip(val);
      }
      return out;
    }
    return x;
  };
  return JSON.stringify(strip(v ?? null));
}
