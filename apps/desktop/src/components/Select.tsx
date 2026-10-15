import { Children, isValidElement, useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { CaretDown, Check } from "@phosphor-icons/react";

type Choice = { value: string; label: ReactNode; disabled?: boolean };
function plain(node: ReactNode): string {
  return Children.toArray(node).map((n) => isValidElement<{ children?: ReactNode }>(n) ? plain(n.props.children) : String(n)).join("");
}
/** A themed, keyboard-accessible single-choice combobox. The popup lives in
 * the document so scroll containers cannot clip it or freeze an OS palette. */
export function Select({ id, value, onValueChange, children, className = "", style, disabled, "aria-label": label }: {
  id?: string; value: string | number; onValueChange: (value: string) => void; children: ReactNode;
  className?: string; style?: CSSProperties; disabled?: boolean; "aria-label"?: string;
}) {
  const uid = useId();
  const options: Choice[] = Children.toArray(children).filter(isValidElement).map((child) => {
    const p = (child as React.ReactElement<{ value: string | number; children: ReactNode; disabled?: boolean }>).props;
    return { value: String(p.value), label: p.children, disabled: p.disabled };
  });
  const index = Math.max(0, options.findIndex((o) => o.value === String(value)));
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(index);
  const [position, setPosition] = useState({ left: 0, top: 0, width: 180, maxHeight: 320 });
  const trigger = useRef<HTMLButtonElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const search = useRef({ text: "", at: 0 });
  const choose = (i: number) => { if (!options[i] || options[i].disabled) return; onValueChange(options[i].value); setOpen(false); trigger.current?.focus(); };
  useLayoutEffect(() => {
    if (!open) return;
    const place = () => {
      const box = trigger.current!.getBoundingClientRect();
      const below = innerHeight - box.bottom - 10;
      const above = box.top - 10;
      const width = Math.min(Math.max(box.width, 224), innerWidth - 12);
      const height = Math.min(320, Math.max(64, Math.max(below, above)));
      const actual = Math.min(height, popup.current?.scrollHeight ?? height);
      setPosition({ left: Math.max(6, Math.min(box.left, innerWidth - width - 6)), top: below < actual && above > below ? box.top - actual - 5 : box.bottom + 5, width, maxHeight: height });
    };
    place();
    addEventListener("resize", place); addEventListener("scroll", place, true);
    return () => { removeEventListener("resize", place); removeEventListener("scroll", place, true); };
  }, [open]);
  useEffect(() => {
    if (!open) return;
    const outside = (e: PointerEvent) => { if (!trigger.current?.contains(e.target as Node) && !popup.current?.contains(e.target as Node)) setOpen(false); };
    const close = () => setOpen(false);
    document.addEventListener("pointerdown", outside, true); addEventListener("blur", close);
    return () => { document.removeEventListener("pointerdown", outside, true); removeEventListener("blur", close); };
  }, [open]);
  useEffect(() => { if (open) popup.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" }); }, [active, open]);
  return <>
    <button ref={trigger} id={id} type="button" role="combobox" aria-label={label} aria-haspopup="listbox" aria-expanded={open} data-value={String(value)}
      aria-controls={open ? uid : undefined} aria-activedescendant={open ? `${uid}-${active}` : undefined} disabled={disabled}
      className={`${className} select-trigger`} style={style} title={plain(options[index]?.label)}
      onClick={() => { setActive(index); setOpen(!open); }} onBlur={(e) => { if (!popup.current?.contains(e.relatedTarget)) setOpen(false); }}
      onKeyDown={(e) => {
        if (e.key === "Escape") { if (open) { e.preventDefault(); e.stopPropagation(); setOpen(false); } return; }
        if (["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) {
          e.preventDefault();
          let next = !open ? index : e.key === "Home" ? 0 : e.key === "End" ? options.length - 1 : (active + (e.key === "ArrowUp" ? -1 : 1) + options.length) % options.length;
          for (let n = 0; n < options.length && options[next]?.disabled; n++) next = (next + 1) % options.length;
          setActive(next); setOpen(true);
        } else if (e.key === "Enter" || e.key === " ") {
          e.preventDefault(); if (open) choose(active); else { setActive(index); setOpen(true); }
        } else if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
          const now = Date.now(); search.current = { at: now, text: (now - search.current.at > 650 ? "" : search.current.text) + e.key.toLowerCase() };
          const next = options.findIndex((o) => !o.disabled && plain(o.label).toLowerCase().startsWith(search.current.text));
          if (next >= 0) { e.preventDefault(); setActive(next); setOpen(true); }
        }
      }}><span>{options[index]?.label}</span><CaretDown size={14} aria-hidden /></button>
    {open && createPortal(<div ref={popup} id={uid} className="select-menu" role="listbox" aria-label={label || "Available options"} style={position}>
      {options.map((option, i) => <div id={`${uid}-${i}`} key={option.value} role="option" aria-selected={option.value === String(value)} aria-disabled={option.disabled || undefined}
        className={`select-option ${i === active ? "active" : ""}`} data-value={option.value} data-index={i}
        onPointerDown={(e) => e.preventDefault()} onPointerMove={() => !option.disabled && setActive(i)} onClick={() => choose(i)}>
        <span>{option.label}</span>{option.value === String(value) && <Check size={14} aria-hidden />}
      </div>)}
    </div>, document.body)}
  </>;
}
