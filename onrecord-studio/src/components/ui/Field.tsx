import {
  cloneElement,
  forwardRef,
  isValidElement,
  useId,
  type InputHTMLAttributes,
  type ReactElement,
  type ReactNode,
  type SelectHTMLAttributes,
  type TextareaHTMLAttributes,
} from 'react';
import { cn } from '../../utils/cn';
import Icon from './icons';

export type FieldSize = 'sm' | 'md';

const CONTROL =
  'w-full border border-control bg-canvas text-control text-fg placeholder:text-muted disabled:bg-raised/50 disabled:text-muted aria-[invalid=true]:border-danger';
const HEIGHT: Record<FieldSize, string> = { sm: 'h-7 rounded-md', md: 'h-8 rounded-lg' };

interface FieldProps {
  label: ReactNode;
  help?: ReactNode;
  /** When set, the control is marked invalid and the message is announced. */
  error?: ReactNode;
  className?: string;
  /** One control; Field supplies its `id`, `aria-describedby` and `aria-invalid`. */
  children: ReactElement<{ id?: string; 'aria-describedby'?: string; 'aria-invalid'?: boolean }>;
}

/** A labelled control with optional helper text and error, associated through generated ids. */
export function Field({ label, help, error, className, children }: FieldProps) {
  const id = useId();
  const helpId = `${id}-help`;
  const errorId = `${id}-error`;
  const controlId = children.props.id ?? id;
  const describedBy = [children.props['aria-describedby'], help ? helpId : null, error ? errorId : null].filter(Boolean).join(' ') || undefined;
  return (
    <div className={cn('flex flex-col gap-1.5', className)}>
      <label htmlFor={controlId} className="text-control text-fg-secondary">{label}</label>
      {isValidElement(children)
        ? cloneElement(children, { id: controlId, 'aria-describedby': describedBy, 'aria-invalid': error ? true : children.props['aria-invalid'] })
        : children}
      {help && !error && <p id={helpId} className="text-caption text-muted">{help}</p>}
      {error && <p id={errorId} role="alert" className="text-caption text-danger">{error}</p>}
    </div>
  );
}

export interface InputProps extends Omit<InputHTMLAttributes<HTMLInputElement>, 'size'> {
  size?: FieldSize;
}

export const Input = forwardRef<HTMLInputElement, InputProps>(function Input({ size = 'md', className, ...props }, ref) {
  return <input ref={ref} className={cn(CONTROL, HEIGHT[size], 'px-3', className)} {...props} />;
});

export const Textarea = forwardRef<HTMLTextAreaElement, TextareaHTMLAttributes<HTMLTextAreaElement>>(function Textarea({ className, ...props }, ref) {
  return <textarea ref={ref} className={cn(CONTROL, 'min-h-20 resize-y rounded-lg px-3 py-2 leading-5', className)} {...props} />;
});

export interface SelectProps extends Omit<SelectHTMLAttributes<HTMLSelectElement>, 'size'> {
  size?: FieldSize;
  /** Visible label. Without it the caller must name the select (`aria-label` or a `Field`). */
  label?: ReactNode;
}

export const Select = forwardRef<HTMLSelectElement, SelectProps>(function Select({ size = 'md', label, className, id, children, ...props }, ref) {
  const generatedId = useId();
  const selectId = id ?? generatedId;
  const control = (
    <div className="relative">
      <select ref={ref} id={selectId} className={cn(CONTROL, HEIGHT[size], 'appearance-none pl-3 pr-8', className)} {...props}>
        {children}
      </select>
      <Icon name="chevron" size="xs" className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 rotate-90 text-muted" />
    </div>
  );
  if (!label) return control;
  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={selectId} className="text-control text-fg-secondary">{label}</label>
      {control}
    </div>
  );
});

export interface SearchFieldProps extends Omit<InputHTMLAttributes<HTMLInputElement>, 'size' | 'type'> {
  size?: FieldSize;
  /** Accessible name; a search field has no visible label. */
  'aria-label': string;
  /** Optional trailing content inside the field, such as a keycap or a clear button. */
  trailing?: ReactNode;
  wrapperClassName?: string;
}

/** A search input with a leading icon. The focus ring is drawn on the wrapper, around the icon. */
export const SearchField = forwardRef<HTMLInputElement, SearchFieldProps>(function SearchField(
  { size = 'md', trailing, wrapperClassName, className, ...props },
  ref,
) {
  return (
    <div className={cn('field flex items-center gap-1.5 border border-control bg-canvas px-3 text-muted', HEIGHT[size], wrapperClassName)}>
      <Icon name="search" size={size === 'sm' ? 'sm' : 'md'} />
      <input
        ref={ref}
        type="search"
        className={cn('h-full min-w-0 flex-1 bg-transparent text-control text-fg placeholder:text-muted [&::-webkit-search-cancel-button]:hidden', className)}
        {...props}
      />
      {trailing}
    </div>
  );
});
