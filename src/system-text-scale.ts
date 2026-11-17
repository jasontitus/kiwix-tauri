export interface SystemTextSize {
  category: string;
  scale: number;
}

/** Shell typography only. Archive text keeps the reader's independent size setting. */
export function applySystemTextScale(
  value: SystemTextSize | null,
  root: Pick<HTMLElement, "style" | "dataset"> | undefined =
    typeof document === "undefined" ? undefined : document.documentElement,
): void {
  if (!root) return;
  // The native UIFontMetrics result includes accessibility categories; do not cap them.
  const scale = value && Number.isFinite(value.scale) && value.scale > 0
    ? value.scale
    : 1;
  root.style.setProperty("--system-text-scale", String(scale));
  root.dataset.contentSizeCategory = value?.category || "default";
  root.dataset.largeText = String(scale > 1.35);
}
