import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { capturePrivacyContext, isPrivacyContextCurrent, subscribeReaderPreferences, type PrivacyContext } from "../api";

const revealEvent = "kiwix-swipe-row-reveal";
const directionThreshold = 12;

type Gesture = {
  pointer: number;
  x: number;
  y: number;
  direction: number;
  initial: number;
  locked: boolean;
  threshold: number;
};

interface SwipeRowHandle {
  identity: symbol;
  close: () => void;
  outside: (event: PointerEvent) => void;
  escape: (event: KeyboardEvent) => void;
}

/** Rows share one document/window listener set instead of adding one each. */
const mountedRows = new Set<SwipeRowHandle>();
/** Subset that can react to a press or scroll: revealed rows and live gestures. */
const armedRows = new Set<SwipeRowHandle>();

function onDocumentPointerDown(event: PointerEvent) {
  for (const row of armedRows) row.outside(event);
}
function onDocumentScroll() {
  for (const row of armedRows) row.close();
}
function onDocumentKeyDown(event: KeyboardEvent) {
  if (event.key !== "Escape") return;
  for (const row of mountedRows) row.escape(event);
}
function onRowReveal(event: Event) {
  const identity = (event as CustomEvent).detail;
  for (const row of mountedRows) {
    if (row.identity !== identity) row.close();
  }
}
function attachRowListeners() {
  document.addEventListener("pointerdown", onDocumentPointerDown, true);
  document.addEventListener("scroll", onDocumentScroll, true);
  document.addEventListener("keydown", onDocumentKeyDown);
  window.addEventListener(revealEvent, onRowReveal);
}
function detachRowListeners() {
  document.removeEventListener("pointerdown", onDocumentPointerDown, true);
  document.removeEventListener("scroll", onDocumentScroll, true);
  document.removeEventListener("keydown", onDocumentKeyDown);
  window.removeEventListener(revealEvent, onRowReveal);
}
function registerRow(row: SwipeRowHandle) {
  mountedRows.add(row);
  if (mountedRows.size === 1) attachRowListeners();
}
function unregisterRow(row: SwipeRowHandle) {
  mountedRows.delete(row);
  armedRows.delete(row);
  if (!mountedRows.size) detachRowListeners();
}

/** Swiping only reveals an explicit action; it never performs that action. */
export function SwipeRow({ children, actionLabel, onAction, disabled = false, className = "" }: {
  children: ReactNode;
  actionLabel: string;
  onAction: () => void;
  disabled?: boolean;
  className?: string;
}) {
  const root = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const identity = useRef(Symbol("swipe-row"));
  const gesture = useRef<Gesture | null>(null);
  const rejectedPointer = useRef<number | null>(null);
  const privacy = useRef<PrivacyContext | null>(null);
  const offset = useRef(0);
  const blocked = useRef(disabled);
  const focusAction = useRef(false);
  const registryEntry = useRef<SwipeRowHandle | null>(null);
  blocked.current = disabled;
  const suppressClickUntil = useRef(0);
  const [distance, setDistance] = useState(0);
  const [open, setOpen] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [direction, setDirection] = useState(1);

  function move(value: number) {
    offset.current = value;
    setDistance(value);
  }
  function close() {
    const active = gesture.current;
    gesture.current = null;
    if (active && root.current?.hasPointerCapture(active.pointer)) {
      root.current.releasePointerCapture(active.pointer);
    }
    const entry = registryEntry.current;
    if (entry) armedRows.delete(entry);
    privacy.current = null;
    focusAction.current = false;
    move(0);
    setOpen(false);
    setDragging(false);
  }
  function authorized() {
    return !blocked.current && privacy.current !== null &&
      !privacy.current.readOnly && isPrivacyContextCurrent(privacy.current);
  }
  function reveal(keyboard = false) {
    if (!authorized()) { close(); return; }
    const entry = registryEntry.current;
    if (entry) armedRows.add(entry);
    window.dispatchEvent(new CustomEvent(revealEvent, { detail: identity.current }));
    move(button.current?.offsetWidth ?? 120);
    focusAction.current = keyboard;
    setOpen(true);
  }

  useEffect(() => {
    const entry: SwipeRowHandle = {
      identity: identity.current,
      close,
      outside(event) {
        // A second contact cancels the gesture, including a pinch starting outside.
        if (!event.isPrimary || (gesture.current && event.pointerId !== gesture.current.pointer)) close();
        if (!root.current?.contains(event.target as Node)) close();
      },
      escape(event) {
        if (event.key === "Escape") {
          if (button.current === document.activeElement) root.current?.focus();
          close();
        }
      },
    };
    registryEntry.current = entry;
    registerRow(entry);
    const unsubscribe = subscribeReaderPreferences(close);
    return () => {
      registryEntry.current = null;
      unregisterRow(entry);
      unsubscribe();
    };
  }, []);
  useEffect(() => { if (disabled) close(); }, [disabled]);
  useEffect(() => {
    if (open && focusAction.current) {
      focusAction.current = false;
      button.current?.focus({ preventScroll: true });
    }
  }, [open]);

  return (
    <div ref={root} className={`swipe-row ${open ? "is-open" : ""} ${dragging ? "is-dragging" : ""} ${className}`}
      style={{ "--swipe-offset": `${-direction * distance}px` } as CSSProperties}
      tabIndex={disabled ? -1 : 0} role="group" aria-label={actionLabel}
      onKeyDown={(event) => {
        if (event.target !== event.currentTarget || disabled) return;
        const rtl = getComputedStyle(event.currentTarget).direction === "rtl";
        if (event.key !== (rtl ? "ArrowRight" : "ArrowLeft")) return;
        event.preventDefault();
        privacy.current = capturePrivacyContext();
        setDirection(rtl ? -1 : 1);
        reveal(true);
      }}
      onPointerDown={(event) => {
        if (disabled || !event.isPrimary || event.button !== 0 ||
          button.current?.contains(event.target as Node) ||
          (event.target as Element).closest("input, textarea, select, summary, [contenteditable]:not([contenteditable=false]), [data-no-swipe]")) return;
        privacy.current = capturePrivacyContext();
        if (!authorized()) { close(); return; }
        const sign = getComputedStyle(event.currentTarget).direction === "rtl" ? -1 : 1;
        setDirection(sign);
        rejectedPointer.current = null;
        gesture.current = { pointer: event.pointerId, x: event.clientX, y: event.clientY,
          direction: sign, initial: offset.current, locked: false,
          threshold: (event.target as Element).closest("button, a, label, [role=button], [role=link], [data-swipe-interactive]") ? 24 : directionThreshold };
        const entry = registryEntry.current;
        if (entry) armedRows.add(entry);
      }}
      onPointerMove={(event) => {
        const active = gesture.current;
        if (!active || event.pointerId !== active.pointer) return;
        if (!authorized()) { close(); return; }
        const dx = event.clientX - active.x;
        const dy = event.clientY - active.y;
        if (!active.locked) {
          if (Math.max(Math.abs(dx), Math.abs(dy)) < directionThreshold) return;
          if (Math.abs(dx) < Math.abs(dy) * 1.3) {
            if (Math.abs(dx) >= active.threshold || active.initial !== 0) {
              rejectedPointer.current = active.pointer;
              suppressClickUntil.current = performance.now() + 700;
            }
            close(); return;
          }
          if (Math.abs(dx) < active.threshold) return;
          if (active.initial === 0 && dx * active.direction > 0) {
            rejectedPointer.current = active.pointer;
            suppressClickUntil.current = performance.now() + 700;
            close(); return;
          }
          active.locked = true;
          suppressClickUntil.current = performance.now() + 700;
          event.currentTarget.setPointerCapture(event.pointerId);
          setDragging(true);
        }
        event.preventDefault();
        const width = button.current?.offsetWidth ?? 120;
        move(Math.min(width, Math.max(0, active.initial - dx * active.direction)));
      }}
      onPointerUp={(event) => {
        if (rejectedPointer.current === event.pointerId) {
          rejectedPointer.current = null;
          suppressClickUntil.current = performance.now() + 700;
        }
        const active = gesture.current;
        if (!active || active.pointer !== event.pointerId) return;
        gesture.current = null;
        setDragging(false);
        const entry = registryEntry.current;
        if (entry) armedRows.delete(entry);
        if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
        if (!active.locked) {
          if (active.initial !== 0) {
            suppressClickUntil.current = performance.now() + 700;
            close();
          }
          return;
        }
        suppressClickUntil.current = performance.now() + 700;
        const width = button.current?.offsetWidth ?? 120;
        if (offset.current >= width * 0.4) reveal();
        else close();
      }}
      onPointerCancel={() => { rejectedPointer.current = null; close(); }}
      onLostPointerCapture={(event) => {
        // Touch starts with implicit capture on the tapped child. Taking capture
        // for this row releases that child; its bubbling event is not our loss.
        if (event.target === event.currentTarget && gesture.current) close();
      }}
      onClickCapture={(event) => {
        if (!button.current?.contains(event.target as Node) && performance.now() < suppressClickUntil.current) {
          event.preventDefault();
          event.stopPropagation();
        }
      }}>
      <button ref={button} type="button" className="swipe-row-action"
        tabIndex={open && !disabled ? 0 : -1} aria-hidden={!open}
        disabled={!open || disabled}
        onClick={(event) => {
          event.stopPropagation();
          const allowed = authorized();
          close();
          if (allowed) onAction();
        }}>{actionLabel}</button>
      <div className="swipe-row-content">{children}</div>
    </div>
  );
}
