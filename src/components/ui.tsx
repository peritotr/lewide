// Small shared pieces from the design's "Shared small components".
import type { ButtonHTMLAttributes, ReactNode } from "react";
import type { Conn, Environment } from "../store";
import { msg, t } from "../i18n";

export const colourVar = (colour: Conn["colour"]) => `var(--c-${colour})`;

const ENV_SHORT: Record<Environment, string> = { production: "PROD", staging: "STG", local: "LOCAL" };

export function EnvTag({ environment, long = false }: { environment: Environment; long?: boolean }) {
  const production = environment === "production";
  return (
    <span
      className={`flex-none border px-[4px] py-[2px] font-heading text-[10px] leading-none font-semibold tracking-[.08em] ${
        production ? "border-warn text-warn-text" : "border-line text-text-2"
      }`}
    >
      {long ? environment.toUpperCase() : ENV_SHORT[environment]}
    </span>
  );
}

/** Filled square = connected, hollow square = disconnected: status is never colour alone. */
export function StatusMark({ connected }: { connected: boolean }) {
  return (
    <i
      title={connected ? t("Connected") : t("Disconnected")}
      className={`size-[7px] flex-none ${connected ? "bg-ok" : "border border-text-3"}`}
    />
  );
}

export function ConnectionMarker({ colour }: { colour: Conn["colour"] }) {
  return <i className="size-[8px] flex-none" style={{ background: colourVar(colour) }} />;
}

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  label: string;
  size?: 18 | 22 | 24;
  children: ReactNode;
}

export function IconButton({ label, size = 24, children, className = "", ...rest }: IconButtonProps) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      className={`grid flex-none place-items-center text-text-2 enabled:hover:bg-hover ${className}`}
      style={{ width: size, height: size }}
      {...rest}
    >
      {children}
    </button>
  );
}

/** Registration marks on the given corners of the nearest `relative` ancestor. */
export function RegMarks({ corners }: { corners: ("tl" | "tr" | "bl" | "br")[] }) {
  return (
    <>
      {corners.map((corner) => (
        <i key={corner} className={`reg reg-${corner}`} />
      ))}
    </>
  );
}

/** Drag handle on a pane border. `onDrag` receives the pointer movement since the last event. */
export function Splitter({
  orientation,
  onDrag,
  className = "",
}: {
  orientation: "vertical" | "horizontal";
  onDrag: (delta: number) => void;
  className?: string;
}) {
  return (
    <div
      role="separator"
      aria-orientation={orientation}
      className={`absolute z-10 ${
        orientation === "vertical" ? "top-0 bottom-0 w-[5px] cursor-col-resize" : "right-0 left-0 h-[5px] cursor-row-resize"
      } ${className}`}
      onPointerDown={(e) => {
        e.preventDefault();
        e.currentTarget.setPointerCapture(e.pointerId);
      }}
      onPointerMove={(e) => {
        if (e.currentTarget.hasPointerCapture(e.pointerId)) {
          onDrag(orientation === "vertical" ? e.movementX : e.movementY);
        }
      }}
    />
  );
}

export const NOT_BUILT = msg("Not built yet");

/** The LewIDE mark (2a "Axis"): two table axes crossing at the corner, and one cell. */
export function Mark({ size }: { size: number }) {
  return (
    <svg viewBox="0 0 32 32" width={size} height={size} fill="none" aria-hidden="true" className="shrink-0">
      <path d="M8 2v27M5 26h25" stroke="var(--text)" strokeWidth="3.5" />
      <rect x="14" y="12" width="8" height="8" fill="var(--accent)" />
    </svg>
  );
}

const TAG = { neutral: "border-line text-text-2", accent: "border-accent-text text-accent-text", danger: "border-danger text-danger", warn: "border-warn text-warn-text" };

/** A small uppercase label in a hairline box. */
export function Tag({ kind = "neutral", children }: { kind?: keyof typeof TAG; children: ReactNode }) {
  return <span className={`flex-none border px-[4px] py-[2px] font-heading text-[10px] leading-none font-semibold tracking-[.08em] whitespace-nowrap uppercase ${TAG[kind]}`}>{children}</span>;
}
