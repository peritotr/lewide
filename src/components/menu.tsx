// Menus and popovers: one surface, anchored to the element that opened it.
import { Check } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";

/** Where to open: under an element's box, or at a point (right-click). */
export type Anchor = DOMRect | { x: number; y: number };

export const anchorOf = (element: Element): Anchor => element.getBoundingClientRect();

interface FloatingProps {
  anchor: Anchor;
  width: number;
  onClose(): void;
  /** Align the right edges instead of the left ones. */
  alignRight?: boolean;
  role?: string;
  label?: string;
  className?: string;
  children: ReactNode;
}

/** A panel below its anchor, kept inside the window. Closes on Escape or a click elsewhere. */
export function Floating({ anchor, width, onClose, alignRight, role = "menu", label, className = "py-s1", children }: FloatingProps) {
  const panel = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    const box = "width" in anchor ? anchor : { left: anchor.x, right: anchor.x, top: anchor.y, bottom: anchor.y };
    const height = panel.current?.offsetHeight ?? 0;
    const left = alignRight ? box.right - width : box.left;
    // Below the anchor when there is room, above it otherwise.
    const below = box.bottom + 2;
    const top = below + height > window.innerHeight - 8 ? Math.max(8, box.top - height - 2) : below;
    setPosition({ left: Math.min(Math.max(8, left), window.innerWidth - width - 8), top });
  }, [anchor, width, alignRight]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    const onDown = (e: MouseEvent) => {
      if (!panel.current?.contains(e.target as Node)) onClose();
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("mousedown", onDown, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("mousedown", onDown, true);
    };
  }, [onClose]);

  return (
    <div
      ref={panel}
      role={role}
      aria-label={label}
      className={`fixed z-40 border border-line bg-panel font-body text-[13px] font-normal text-text ${className}`}
      style={{ width, left: position?.left ?? -9999, top: position?.top ?? -9999, boxShadow: "var(--shadow-md)" }}
      onContextMenu={(e) => e.preventDefault()}
    >
      {children}
    </div>
  );
}

interface ItemProps {
  onSelect?(): void;
  /** Right-aligned shortcut or note. */
  hint?: ReactNode;
  /** In a list where one item is chosen: true shows the check, false keeps the indent. */
  checked?: boolean;
  danger?: boolean;
  disabled?: boolean;
  title?: string;
  children: ReactNode;
}

export function MenuItem({ onSelect, hint, checked, danger, disabled, title, children }: ItemProps) {
  return (
    <button
      type="button"
      role={checked === undefined ? "menuitem" : "menuitemradio"}
      aria-checked={checked}
      disabled={disabled}
      title={title}
      className={`flex min-h-[26px] w-full items-center gap-s2 px-s3 text-left ${checked ? "font-medium" : ""} ${
        danger ? "text-danger enabled:hover:bg-danger-tint" : "enabled:hover:bg-accent-tint"
      }`}
      onClick={onSelect}
    >
      {checked !== undefined && <span className="grid w-[12px] flex-none place-items-center">{checked && <Check size={12} />}</span>}
      <span className="min-w-0 flex-1">{children}</span>
      {hint && <span className="flex-none font-mono text-[11px] text-text-3">{hint}</span>}
    </button>
  );
}

export const MenuSeparator = () => <i className="my-s1 block h-px bg-line" />;
