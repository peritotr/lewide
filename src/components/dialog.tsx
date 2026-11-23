// Dialog and form building blocks from the connections design ("Shared building blocks").
import { Check, X } from "lucide-react";
import { useEffect, useRef, type ButtonHTMLAttributes, type InputHTMLAttributes, type ReactNode } from "react";
import { IconButton, RegMarks } from "./ui";
import { t } from "../i18n";

const FOCUSABLE = 'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex="0"]';

interface DialogProps {
  title: ReactNode;
  /** Shown after the title in the header. */
  headerExtra?: ReactNode;
  width: number;
  /** Extra distance from the top, on top of the shared 96px. */
  offset?: number;
  onClose(): void;
  /** Hatched band under the header: Production, or danger for a security warning. */
  band?: "production" | "danger";
  alert?: boolean;
  danger?: boolean;
  footer: ReactNode;
  /** Rendered between body and footer, outside the body's padding. */
  beforeFooter?: ReactNode;
  children: ReactNode;
  bodyClassName?: string;
  onSubmit?(): void;
  /** Above another dialog. */
  nested?: boolean;
}

/** Modal dialog: top-anchored so its header stays put when the height changes. */
export function Dialog(props: DialogProps) {
  const panel = useRef<HTMLFormElement>(null);

  // Focus goes into the dialog on open and back to where it was on close.
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const first = panel.current?.querySelector<HTMLElement>("[data-autofocus]") ?? panel.current?.querySelector<HTMLElement>(FOCUSABLE);
    first?.focus();
    return () => previous?.focus?.();
  }, []);

  // Escape also closes when focus has left the dialog, for instance after the
  // focused button was replaced by another step's.
  const onClose = useRef(props.onClose);
  onClose.current = props.onClose;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      const active = document.activeElement;
      if (!active || active === document.body || !active.closest('[role="dialog"], [role="alertdialog"], [role="menu"]')) {
        e.preventDefault();
        onClose.current();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  function onKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Escape") {
      e.stopPropagation();
      props.onClose();
    } else if (e.key === "Tab" && panel.current) {
      // Keep Tab inside the dialog.
      const items = [...panel.current.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => el.offsetParent !== null);
      const first = items[0];
      const last = items[items.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last?.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first?.focus();
      }
    }
  }

  return (
    <div
      className={`fixed inset-0 flex items-start justify-center overflow-y-auto bg-scrim ${props.nested ? "z-[60]" : "z-50"}`}
      style={{ paddingTop: 96 + (props.offset ?? 0), paddingBottom: 24 }}
      onMouseDown={(e) => e.target === e.currentTarget && props.onClose()}
      onKeyDown={onKeyDown}
    >
      <form
        ref={panel}
        role={props.alert ? "alertdialog" : "dialog"}
        aria-modal="true"
        className={`flex max-w-[calc(100vw-32px)] flex-col border bg-bg ${props.danger ? "border-danger" : "border-line"}`}
        style={{ width: props.width, boxShadow: "var(--shadow-lg)" }}
        onSubmit={(e) => {
          e.preventDefault();
          props.onSubmit?.();
        }}
      >
        {props.band === "danger" && <div className="danger-band h-[4px]" />}
        <header className="flex h-[44px] flex-none items-center gap-s3 border-b border-line pr-s2 pl-[20px]">
          <h2 className="font-heading text-[18px] leading-none font-semibold whitespace-nowrap">{props.title}</h2>
          {props.headerExtra}
          <span className="flex-1" />
          <IconButton label={t("Close")} onClick={props.onClose}>
            <X size={14} />
          </IconButton>
        </header>
        {props.band === "production" && <div title={t("Production connection")} className="production-band h-[4px] flex-none" />}
        <div className={props.bodyClassName ?? "p-[20px]"}>{props.children}</div>
        {props.beforeFooter}
        {props.footer != null && <footer className="flex h-[52px] flex-none items-center justify-end gap-s2 border-t border-line px-[20px]">{props.footer}</footer>}
      </form>
    </div>
  );
}

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & { variant?: "primary" | "secondary" | "ghost" | "danger"; tall?: boolean };

export function Button({ variant = "secondary", tall = false, className = "", children, type = "button", ...rest }: ButtonProps) {
  const height = tall ? "h-[32px]" : "h-[28px]";
  const look = {
    primary: "relative bg-accent-fill px-s4 font-heading text-[13px] font-semibold tracking-[.06em] text-on-accent uppercase",
    danger: "bg-danger px-s4 font-heading text-[13px] font-semibold tracking-[.06em] text-on-danger uppercase",
    secondary: "border border-line px-s3 enabled:hover:bg-hover",
    ghost: "px-s3 enabled:hover:bg-hover",
  }[variant];
  return (
    <button type={type} className={`flex flex-none items-center gap-[6px] whitespace-nowrap ${height} ${look} ${className}`} {...rest}>
      {variant === "primary" && <RegMarks corners={["tl", "tr", "bl", "br"]} />}
      {children}
    </button>
  );
}

export function Field({ label, helper, error, children, className = "" }: { label: string; helper?: ReactNode; error?: string; children: ReactNode; className?: string }) {
  return (
    <label className={`flex min-w-0 flex-col gap-[4px] ${className}`}>
      <span className="text-[12px] text-text-2">{label}</span>
      {children}
      {error && <span className="text-[12px] text-danger">{error}</span>}
      {helper && !error && <span className="pt-[2.8px] text-[12px] text-text-3">{helper}</span>}
    </label>
  );
}

type InputProps = InputHTMLAttributes<HTMLInputElement> & { mono?: boolean; invalid?: boolean; before?: ReactNode; after?: ReactNode };

export function TextInput({ mono = false, invalid = false, before, after, className = "", ...rest }: InputProps) {
  return (
    <span
      aria-disabled={rest.disabled}
      className={`flex h-[28px] min-w-0 items-center gap-s2 border bg-panel px-[8px] focus-within:border-accent focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-accent ${
        invalid ? "border-danger" : "border-line"
      } ${className}`}
    >
      {before}
      <input
        autoCapitalize="off"
        autoCorrect="off"
        spellCheck={false}
        className={`min-w-0 flex-1 bg-transparent text-text placeholder:text-text-3 focus:outline-none disabled:cursor-not-allowed ${mono ? "font-mono text-[12.5px]" : "text-[13px]"}`}
        {...rest}
      />
      {after}
    </span>
  );
}

export function Checkbox({ checked, onChange, disabled, children, autoFocus }: { checked: boolean; onChange(checked: boolean): void; disabled?: boolean; children: ReactNode; autoFocus?: boolean }) {
  return (
    <label aria-disabled={disabled} className="flex items-center gap-s2 text-[12.5px]">
      <span className="relative grid size-[14px] flex-none place-items-center">
        <input
          type="checkbox"
          checked={checked}
          disabled={disabled}
          autoFocus={autoFocus}
          onChange={(e) => onChange(e.target.checked)}
          className="peer absolute inset-0 cursor-pointer appearance-none border border-text-2 checked:border-accent-fill checked:bg-accent-fill disabled:cursor-not-allowed"
        />
        <Check size={10} strokeWidth={2.5} className="pointer-events-none relative hidden text-on-accent peer-checked:block" />
      </span>
      {children}
    </label>
  );
}

export function Radio({ checked, onChange, title, children }: { checked: boolean; onChange(): void; title: string; children: ReactNode }) {
  return (
    <label className="flex cursor-pointer items-start gap-s2 py-[5px]">
      <span className="relative mt-[2px] grid size-[14px] flex-none place-items-center">
        <input
          type="radio"
          checked={checked}
          onChange={onChange}
          className="peer absolute inset-0 cursor-pointer appearance-none rounded-full border border-text-2 checked:border-accent-fill"
        />
        <i className="relative hidden size-[8px] rounded-full bg-accent-fill peer-checked:block" />
      </span>
      <span>
        <span className="font-medium">{title}</span>
        <span className="text-text-2"> — {children}</span>
      </span>
    </label>
  );
}

export function Switch({ checked, onChange, children }: { checked: boolean; onChange(checked: boolean): void; children: ReactNode }) {
  return (
    <label className="flex cursor-pointer items-center gap-s2">
      <span className="font-medium">{children}</span>
      <span className={`relative h-[16px] w-[28px] flex-none border ${checked ? "border-accent-fill bg-accent-fill" : "border-text-2"}`}>
        <input type="checkbox" role="switch" checked={checked} onChange={(e) => onChange(e.target.checked)} className="absolute inset-0 cursor-pointer opacity-0" />
        <i className={`pointer-events-none absolute top-[1px] size-[12px] ${checked ? "right-[1px] bg-on-accent" : "left-[1px] bg-text-2"}`} />
      </span>
      <span className="text-[12px] text-text-2">{checked ? t("On") : t("Off")}</span>
    </label>
  );
}

export function Segmented<T extends string>({ value, onChange, options, label, disabled }: {
  value: T;
  onChange(value: T): void;
  options: { value: T; label: string; warn?: boolean; disabled?: boolean; title?: string }[];
  label: string;
  disabled?: boolean;
}) {
  return (
    <div role="radiogroup" aria-label={label} aria-disabled={disabled} className="flex h-[28px] w-fit border border-line text-[12.5px]">
      {options.map((option, i) => {
        const selected = option.value === value;
        const look = !selected
          ? "enabled:hover:bg-hover"
          : option.warn
            ? "bg-warn-tint font-medium text-warn-text"
            : "bg-accent-fill font-medium text-on-accent";
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={selected}
            disabled={disabled || option.disabled}
            title={option.title}
            className={`flex items-center gap-[5px] px-[10px] disabled:cursor-not-allowed disabled:text-text-3 whitespace-nowrap ${i > 0 ? "border-l border-line" : ""} ${look}`}
            style={selected && option.warn ? { boxShadow: "inset 0 0 0 1px var(--warn)" } : undefined}
            onClick={() => onChange(option.value)}
          >
            {selected && option.warn && <Check size={12} />}
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

const STRIP = {
  warning: "border-warn bg-warn-tint [&>svg]:text-warn-text",
  error: "border-danger bg-danger-tint [&>svg]:text-danger",
  success: "border-ok bg-ok-tint [&>svg]:text-ok",
  neutral: "border-line",
  note: "border-line bg-accent-tint [&>svg]:text-accent-text",
};

/** Inline message: an icon, text, and optional actions on the right. */
export function Strip({ kind, icon, children, className = "" }: { kind: keyof typeof STRIP; icon?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <div role={kind === "error" ? "alert" : "status"} className={`flex items-start gap-s2 border px-s3 py-s2 text-[12.5px] [&>svg]:mt-[1px] [&>svg]:flex-none ${STRIP[kind]} ${className}`}>
      {icon}
      <div className="min-w-0 flex-1">{children}</div>
    </div>
  );
}

/** Two-letter database type badge as used in dialogs. */
export function TypeBadge({ code, dashed = false }: { code: string; dashed?: boolean }) {
  return (
    <span className={`grid h-[18px] w-[26px] flex-none place-items-center border border-text-3 font-mono text-[10.5px] leading-none font-medium ${dashed ? "border-dashed border-line text-text-3" : ""}`}>
      {code}
    </span>
  );
}

export const linkButton = "text-accent-text hover:underline";
