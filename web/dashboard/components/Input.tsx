import { useId, useState } from "react";
import type { InputHTMLAttributes, ReactNode, SelectHTMLAttributes, TextareaHTMLAttributes } from "react";
import { AlertCircleIcon, ChevronDownIcon, EyeIcon, EyeOffIcon } from "./icons";

/** Shared by every labelled control. Label is always visible unless hideLabel (then sr-only). */
export interface FieldProps {
  label?: ReactNode;
  /** Gray text under the control. Replaced by `error` when both are set. */
  helper?: ReactNode;
  /** Error message; turns label + border red and sets aria-invalid. `true` = invalid without a message (alert shown elsewhere). */
  error?: ReactNode;
  /** Adds a gray "(optional)" after the label. */
  optional?: boolean;
  hideLabel?: boolean;
  /** Class for the outer wrapper (label + control + helper). */
  wrapperClassName?: string;
}

function Field({
  id,
  label,
  helper,
  error,
  optional,
  hideLabel,
  disabled,
  wrapperClassName = "",
  children,
}: FieldProps & { id: string; disabled?: boolean; children: ReactNode }) {
  return (
    <div className={`flex min-w-0 flex-col gap-1.5 ${wrapperClassName}`}>
      {label != null && (
        <label
          htmlFor={id}
          className={`${hideLabel ? "sr-only" : ""} text-13 font-semibold ${error ? "text-danger" : disabled ? "text-ink-3" : "text-ink"}`}
        >
          {label}
          {optional && <span className="font-normal text-ink-3"> (optional)</span>}
        </label>
      )}
      {children}
      {error && error !== true ? (
        <div id={`${id}-desc`} className="flex items-center gap-1.5 text-xs text-danger">
          <AlertCircleIcon size={14} strokeWidth={2.25} />
          {error}
        </div>
      ) : helper && !error ? (
        <div id={`${id}-desc`} className="text-xs text-ink-3">
          {helper}
        </div>
      ) : null}
    </div>
  );
}

const CONTROL =
  "w-full rounded-lg bg-surface text-sm text-ink placeholder:text-ink-3 focus:border-info focus-visible:outline-offset-1 " +
  "disabled:cursor-not-allowed disabled:border-line disabled:bg-sunken disabled:text-ink-3";
const border = (error: unknown) => (error ? "border-[1.5px] border-danger" : "border border-line-strong");

function a11y(id: string, f: FieldProps) {
  return {
    id,
    "aria-invalid": f.error ? true : undefined,
    "aria-describedby": (f.error && f.error !== true) || f.helper ? `${id}-desc` : undefined,
  };
}

function splitField<T extends FieldProps>(p: T) {
  const { label, helper, error, optional, hideLabel, wrapperClassName, ...rest } = p;
  return [{ label, helper, error, optional, hideLabel, wrapperClassName }, rest] as const;
}

export interface TextFieldProps extends FieldProps, Omit<InputHTMLAttributes<HTMLInputElement>, "children"> {
  /** Mono text for codes, ids, URLs. */
  mono?: boolean;
}

/** 44px text input with label, helper and error. `className` goes on the <input>. */
export function TextField(props: TextFieldProps) {
  const [f, { mono, className = "", id: idProp, ...rest }] = splitField(props);
  const auto = useId();
  const id = idProp ?? auto;
  return (
    <Field {...f} id={id} disabled={rest.disabled}>
      <input
        {...rest}
        {...a11y(id, f)}
        className={`${CONTROL} ${border(f.error)} h-11 px-3 ${mono ? "font-mono" : ""} ${className}`}
      />
    </Field>
  );
}

/** Backwards-compatible name used by existing pages. */
export const Input = TextField;

/** Password input with a show / hide eye button inside the field. */
export function PasswordField(props: Omit<TextFieldProps, "type">) {
  const [f, { mono, className = "", id: idProp, ...rest }] = splitField(props);
  const auto = useId();
  const id = idProp ?? auto;
  const [show, setShow] = useState(false);
  return (
    <Field {...f} id={id} disabled={rest.disabled}>
      <div
        className={`flex h-11 items-center rounded-lg pl-3 ${border(f.error)} ${rest.disabled ? "border-line bg-sunken" : "bg-surface"} has-[input:focus-visible]:border-info has-[input:focus-visible]:outline has-[input:focus-visible]:outline-2 has-[input:focus-visible]:outline-offset-1 has-[input:focus-visible]:outline-info`}
      >
        <input
          {...rest}
          {...a11y(id, f)}
          type={show ? "text" : "password"}
          className={`h-full min-w-0 flex-1 bg-transparent text-sm text-ink placeholder:text-ink-3 focus-visible:outline-none disabled:cursor-not-allowed disabled:text-ink-3 ${className}`}
        />
        <button
          type="button"
          onClick={() => setShow((s) => !s)}
          aria-label={show ? "Hide password" : "Show password"}
          aria-pressed={show}
          disabled={rest.disabled}
          className="inline-flex h-[42px] w-11 shrink-0 items-center justify-center rounded-r-lg text-ink-2 hover:text-ink focus-visible:outline-offset-[-2px]"
        >
          {show ? <EyeOffIcon /> : <EyeIcon />}
        </button>
      </div>
    </Field>
  );
}

export interface TextareaProps extends FieldProps, Omit<TextareaHTMLAttributes<HTMLTextAreaElement>, "children"> {}

export function Textarea(props: TextareaProps) {
  const [f, { className = "", id: idProp, rows = 3, ...rest }] = splitField(props);
  const auto = useId();
  const id = idProp ?? auto;
  return (
    <Field {...f} id={id} disabled={rest.disabled}>
      <textarea {...rest} {...a11y(id, f)} rows={rows} className={`${CONTROL} ${border(f.error)} resize-y px-3 py-2.5 ${className}`} />
    </Field>
  );
}

export interface SelectOptionItem {
  value: string | number;
  label: string;
}

export interface SelectProps extends FieldProps, Omit<SelectHTMLAttributes<HTMLSelectElement>, "children"> {
  options?: SelectOptionItem[];
  /** Renders a first option with value "" (e.g. "Any language"). */
  placeholder?: string;
  children?: ReactNode;
}

/** Native <select>, styled. Use SearchableSelect when the list is long or remote. */
export function Select(props: SelectProps) {
  const [f, { className = "", id: idProp, options, placeholder, children, ...rest }] = splitField(props);
  const auto = useId();
  const id = idProp ?? auto;
  return (
    <Field {...f} id={id} disabled={rest.disabled}>
      <div className="relative">
        <select {...rest} {...a11y(id, f)} className={`${CONTROL} ${border(f.error)} h-11 appearance-none pl-3 pr-10 ${className}`}>
          {placeholder != null && <option value="">{placeholder}</option>}
          {options?.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
          {children}
        </select>
        <ChevronDownIcon size={16} className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-ink-2" />
      </div>
    </Field>
  );
}

/** On/off switch (role="switch"). Green when on. 44px tall hit area. */
export function Toggle({
  checked,
  onChange,
  label,
  disabled,
  id,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: ReactNode;
  disabled?: boolean;
  id?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      id={id}
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className="inline-flex min-h-11 items-center gap-2.5 rounded-lg text-sm font-medium text-ink disabled:cursor-not-allowed disabled:text-ink-3"
    >
      <span className={`relative inline-block h-6 w-10 shrink-0 rounded-full transition-colors ${checked ? "bg-success" : "bg-line-strong"} ${disabled ? "opacity-60" : ""}`}>
        <span className={`absolute top-[3px] h-[18px] w-[18px] rounded-full bg-white transition-[left] ${checked ? "left-[19px]" : "left-[3px]"}`} />
      </span>
      {label}
    </button>
  );
}

/** Native checkbox with label, 44px tall hit area. */
export function Checkbox({ label, className = "", ...rest }: Omit<InputHTMLAttributes<HTMLInputElement>, "type"> & { label: ReactNode }) {
  return (
    <label className={`inline-flex min-h-11 cursor-pointer items-center gap-2.5 text-sm font-medium text-ink ${className}`}>
      <input type="checkbox" {...rest} className="m-0 h-[18px] w-[18px] accent-ink" />
      {label}
    </label>
  );
}
