import { useEffect, useLayoutEffect, useRef, useState } from "react";

// 原位文本编辑（计划 §9.3）。
//
// 不使用裸 contentEditable + document.execCommand。原因（计划 §9.3 列出的实际故障）：
// 中文输入法 composition 与 React rerender 冲突、光标跳动、粘贴富文本污染、
// DOM 与 React 状态分叉、浏览器 undo 与应用 undo 不一致、blur 前崩溃丢内容。
//
// 这里用一个继承字体/字号/行高、无边框的 auto-size textarea：视觉上仍是原位编辑，
// 聚焦时只多一个淡色 focus ring；学生模式的 DOM 里完全没有 textarea。
export function InlineTextEditor({
  value,
  ariaLabel,
  className,
  onCommit,
  onCancel
}: {
  value: string;
  ariaLabel: string;
  className?: string;
  onCommit: (next: string) => void;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState(value);
  const textarea = useRef<HTMLTextAreaElement>(null);
  // 输入法组合期间不提交，也不让外部 value 覆盖草稿。
  const composing = useRef(false);
  const committed = useRef(false);

  useLayoutEffect(() => {
    const element = textarea.current;
    if (!element) return;
    element.focus();
    element.setSelectionRange(element.value.length, element.value.length);
  }, []);

  // 高度跟随内容，避免出现内部滚动条把长题干截断。
  useLayoutEffect(() => {
    const element = textarea.current;
    if (!element) return;
    element.style.height = "auto";
    element.style.height = `${element.scrollHeight}px`;
  }, [draft]);

  useEffect(() => {
    if (composing.current) return;
    setDraft(value);
  }, [value]);

  // 提交时读取 DOM 里的实时值，而不是依赖 draft 闭包：输入和失焦落在同一批 React 更新里时
  // （快速输入后立刻点走），闭包里的 draft 还是上一次渲染的旧值，会静默丢掉最后一次输入。
  const commit = (nextValue?: string) => {
    if (committed.current || composing.current) return;
    committed.current = true;
    const next = nextValue ?? textarea.current?.value ?? draft;
    if (next === value) onCancel();
    else onCommit(next);
  };

  return (
    <textarea
      ref={textarea}
      className={`inline-text-editor ${className ?? ""}`.trim()}
      value={draft}
      rows={1}
      aria-label={ariaLabel}
      spellCheck={false}
      onChange={(event) => setDraft(event.target.value)}
      onCompositionStart={() => {
        composing.current = true;
      }}
      onCompositionEnd={(event) => {
        composing.current = false;
        setDraft(event.currentTarget.value);
      }}
      onPaste={(event) => {
        // 只接受纯文本，避免把富文本样式带进 ContentDoc。
        event.preventDefault();
        const text = event.clipboardData.getData("text/plain");
        const element = event.currentTarget;
        const start = element.selectionStart ?? draft.length;
        const end = element.selectionEnd ?? start;
        const next = `${draft.slice(0, start)}${text}${draft.slice(end)}`;
        setDraft(next);
        requestAnimationFrame(() => {
          const caret = start + text.length;
          element.setSelectionRange(caret, caret);
        });
      }}
      onKeyDown={(event) => {
        if (composing.current) return;
        if (event.key === "Escape") {
          event.preventDefault();
          committed.current = true;
          onCancel();
          return;
        }
        // Enter 提交；Shift+Enter 留给多行题干。
        if (event.key === "Enter" && !event.shiftKey) {
          event.preventDefault();
          commit(event.currentTarget.value);
        }
      }}
      onBlur={(event) => commit(event.currentTarget.value)}
    />
  );
}
