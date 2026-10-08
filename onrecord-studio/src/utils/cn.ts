import { clsx, type ClassValue } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

// tailwind-merge only knows Tailwind's default theme. Without these lists it reads a custom size
// such as `text-caption` as a text colour and drops it whenever a colour class follows. Keep each
// list in sync with the matching key in tailwind.config.js. `control` is a border and fill colour
// only: `text-control` is the control type size, so it is not in the shared colour list.
const twMerge = extendTailwindMerge({
  extend: {
    classGroups: {
      "font-size": [{ text: ["banner", "display", "title", "rank", "dek", "heading", "headline", "body", "label", "control", "caption", "overline", "metric", "code"] }],
      "border-color": [{ border: ["control"] }],
      "bg-color": [{ bg: ["control"] }],
      shadow: [{ shadow: ["panel", "popover", "dialog"] }],
      "max-w": [{ "max-w": ["content", "prose"] }],
      z: [{ z: ["sticky", "header", "overlay", "popover", "sheet", "dialog", "palette", "toast", "gate"] }],
    },
    theme: {
      colors: [
        "fg", "fg-secondary", "muted", "canvas", "surface", "raised", "line", "line-strong",
        "accent", "on-accent", "violation", "on-violation", "highlight", "success", "warning", "danger", "focus", "scrim", "hatch",
        "panel", "chrome", "bg", "lead", "lead-mark", "on-lead", "success-mark", "danger-mark", "on-danger",
        "warning-mark", "viz-surface", "viz-ink", "viz-mark", "viz-context", "viz-track", "viz-grid", "viz-baseline",
        "viz-lead", "viz-positive", "viz-counted",
      ],
    },
  },
});

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
