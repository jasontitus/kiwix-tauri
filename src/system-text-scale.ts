export interface SystemTextSize {
  category: string;
  scale: number;
}

/** The category the Android plugin reports with the system font scale. */
export const ANDROID_TEXT_CATEGORY = "android";

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
  // Android's WebView already multiplies every text size by the system font
  // scale. Scaling the shell's own sizes as well would apply it twice, so on
  // Android the scale only selects the large-text layout below.
  const typography = value?.category === ANDROID_TEXT_CATEGORY ? 1 : scale;
  root.style.setProperty("--system-text-scale", String(typography));
  root.dataset.contentSizeCategory = value?.category || "default";
  root.dataset.largeText = String(scale > 1.35);
}
