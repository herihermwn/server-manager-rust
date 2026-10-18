import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Button, IconButton } from "./Button";
import { DotsIcon, TrashIcon } from "./icons";

/** "More actions" menu of a database row. Rendered in a portal: the table scrolls sideways and would clip it. */
export function DbRowMenu({ name, onDrop }: { name: string; onDrop: () => void }) {
  const [pos, setPos] = useState<{ top: number; right: number } | null>(null);
  const trigger = useRef<HTMLSpanElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const close = () => setPos(null);

  useEffect(() => {
    if (!pos) return;
    const onDown = (e: MouseEvent) => {
      if (!menu.current?.contains(e.target as Node) && !trigger.current?.contains(e.target as Node)) close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        close();
        trigger.current?.querySelector("button")?.focus();
      }
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    window.addEventListener("scroll", close, true);
    window.addEventListener("resize", close);
    menu.current?.querySelector("button")?.focus();
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("resize", close);
    };
  }, [pos]);

  return (
    <>
      <span ref={trigger} className="inline-flex">
      <IconButton
        label={`More actions for ${name}`}
        variant="ghost"
        size="sm"
        aria-haspopup="menu"
        aria-expanded={!!pos}
        onClick={() => {
          const r = trigger.current!.getBoundingClientRect();
          setPos(pos ? null : { top: r.bottom + 4, right: window.innerWidth - r.right });
        }}
      >
        <DotsIcon size={16} />
      </IconButton>
      </span>
      {pos &&
        createPortal(
          <div
            ref={menu}
            role="menu"
            style={{ position: "fixed", top: pos.top, right: pos.right }}
            className="z-40 rounded-xl border border-line bg-surface p-1.5 shadow-overlay"
          >
            <Button
              role="menuitem"
              variant="danger"
              icon={<TrashIcon size={16} />}
              onClick={() => {
                close();
                onDrop();
              }}
            >
              Drop…
            </Button>
          </div>,
          document.body,
        )}
    </>
  );
}
