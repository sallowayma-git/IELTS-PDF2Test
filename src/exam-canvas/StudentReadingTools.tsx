import { useEffect, useRef, useState, type RefObject } from "react";
import { wrapRangeTextWithHighlight } from "./highlightDom";

export type StudentFontSize = "normal" | "large" | "xlarge";

export function StudentFontSizeControls({ value, onChange }: { value: StudentFontSize; onChange: (size: StudentFontSize) => void }) {
  return <div className="student-font-controls" role="group" aria-label="Text size">
    {(["normal", "large", "xlarge"] as const).map((size, index) => <button type="button" key={size} aria-label={`Text size ${size}`} aria-pressed={value === size} onClick={() => onChange(size)}>{["A−", "A", "A+"][index]}</button>)}
  </div>;
}

/** Preview-only decoration: never mutates authoring, answers, or saved drafts. */
export function StudentReadingTools({ root, fontSize, onFontSizeChange }: {
  root: RefObject<HTMLDivElement | null>; fontSize: StudentFontSize; onFontSizeChange?: (size: StudentFontSize) => void;
}) {
  const rangeRef = useRef<Range | null>(null);
  const activeRef = useRef<HTMLElement | null>(null);
  const [toolbar, setToolbar] = useState<{ top: number; left: number; canRemove: boolean }>();
  useEffect(() => {
    const container = root.current;
    if (!container) return;
    const pane = (node: Node) => (node.nodeType === Node.TEXT_NODE ? node.parentElement : node as Element)?.closest(".reading-pane");
    const show = (rect: DOMRect, canRemove: boolean) => setToolbar({ top: Math.max(8, rect.top < 80 ? rect.bottom + 8 : rect.top - 42), left: Math.max(8, Math.min(rect.left, window.innerWidth - 240)), canRemove });
    const selected = () => {
      const selection = window.getSelection();
      if (!selection?.rangeCount || selection.isCollapsed) return;
      const range = selection.getRangeAt(0);
      const start = pane(range.startContainer);
      if (!start || !container.contains(start) || start !== pane(range.endContainer)) { setToolbar(undefined); return; }
      rangeRef.current = range.cloneRange();
      activeRef.current = null;
      show(range.getBoundingClientRect(), [...start.querySelectorAll<HTMLElement>(".hl")].some((el) => range.intersectsNode(el)));
    };
    const clicked = (event: MouseEvent) => {
      const target = event.target as Element;
      if (target.closest(".student-highlight-toolbar")) return;
      const highlight = target.closest<HTMLElement>(".hl");
      if (highlight && container.contains(highlight)) { activeRef.current = highlight; rangeRef.current = null; show(highlight.getBoundingClientRect(), true); }
      else if (window.getSelection()?.isCollapsed) setToolbar(undefined);
    };
    document.addEventListener("selectionchange", selected);
    document.addEventListener("click", clicked);
    container.addEventListener("mouseup", selected);
    const hide = () => setToolbar(undefined);
    container.addEventListener("scroll", hide, true);
    return () => {
      document.removeEventListener("selectionchange", selected);
      document.removeEventListener("click", clicked);
      container.removeEventListener("mouseup", selected);
      container.removeEventListener("scroll", hide, true);
    };
  }, [root]);
  const finish = () => { window.getSelection()?.removeAllRanges(); rangeRef.current = null; activeRef.current = null; setToolbar(undefined); };
  return <>
    {onFontSizeChange ? <div className="student-reading-toolbar"><StudentFontSizeControls value={fontSize} onChange={onFontSizeChange} /></div> : null}
    {toolbar ? <div className="student-highlight-toolbar" role="toolbar" aria-label="Highlight selection" style={{ position: "fixed", top: toolbar.top, left: toolbar.left }} onPointerDown={(event) => event.preventDefault()}>
      <button type="button" disabled={!!activeRef.current} onClick={() => { if (rangeRef.current) wrapRangeTextWithHighlight(rangeRef.current, { kind: "highlight" }); finish(); }}>Highlight</button>
      <button type="button" disabled={!toolbar.canRemove} onClick={() => {
        const range = rangeRef.current;
        const targets = activeRef.current ? [activeRef.current] : [...root.current!.querySelectorAll<HTMLElement>(".hl")].filter((el) => range?.intersectsNode(el));
        targets.forEach((el) => el.replaceWith(...el.childNodes));
        finish();
      }}>Remove highlight</button>
    </div> : null}
  </>;
}
