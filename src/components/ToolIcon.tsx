import type { Category } from "@/domain/types";
import { CATEGORY_ICON } from "@/presentation/labels";

/** Category glyph in a tile. Tool logos are deliberately not bundled. */
export function ToolIcon({ category, size = 36 }: { category: Category; size?: number }) {
  const Icon = CATEGORY_ICON[category];
  return (
    <span
      aria-hidden
      style={{ width: size, height: size }}
      className="flex shrink-0 items-center justify-center rounded-md bg-muted text-primary"
    >
      <Icon size={size * 0.5} />
    </span>
  );
}
