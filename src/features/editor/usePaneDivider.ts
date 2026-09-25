import { useCallback, useEffect, useRef, useState } from "react";
// React 的事件类型带 `Element` 泛型，与 window 上监听的原生 DOM PointerEvent 同名不同型；
// 起别名区分：ReactXxx 用于 JSX 属性回调，原生 PointerEvent 用于 window 监听器。
import type { KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent } from "react";

/**
 * 题面「原文 | 题目」两栏的可拖动分隔条（基准页 #divider 的行为）。
 *
 * 由 ExamCanvas 在**两种模式**下渲染同一个分隔条元素：
 *   - 编辑模式：`.workspace-body > .exam-canvas-v2` 是 display:contents，
 *     分隔条成为 `.workspace-body` 三列网格的第 2 列；
 *   - 学生预览：分隔条是 `.workspace-student-preview > .exam-canvas-v2`
 *     三列网格的第 2 列。
 * 两种模式的网格都引用 `--workspace-left-pane-width`（定义在 .workspace-body 上，
 * 学生预览容器从它继承），所以本 hook 只需要改写 .workspace-body 上的这个变量。
 *
 * 宽度按条目记入 localStorage（读写都包 try/catch），范围钳制在
 * 320px ~ (容器宽 - 320px)；支持左右方向键微调；拖动期间给 body 加
 * .workspace-resizing（workspace.css 据此强制 col-resize 光标并禁止选中）。
 */

const MIN_PANE_PX = 320;
const KEYBOARD_STEP_PX = 24;

export interface PaneDividerProps {
  role: "separator";
  "aria-orientation": "vertical";
  "aria-label": string;
  tabIndex: 0;
  className: string;
  onPointerDown: (event: ReactPointerEvent<HTMLDivElement>) => void;
  onKeyDown: (event: ReactKeyboardEvent<HTMLDivElement>) => void;
}

export interface PaneDividerHandle {
  dividerProps: PaneDividerProps;
  isDragging: boolean;
}

interface DragState {
  startX: number;
  startWidth: number;
}

function storageKey(itemId: string): string {
  return `pdf2test.workspace.pane-width.${itemId}`;
}

function clampWidth(px: number, container: HTMLElement): number {
  const max = Math.max(MIN_PANE_PX, container.clientWidth - MIN_PANE_PX);
  return Math.min(Math.max(px, MIN_PANE_PX), max);
}

/** 分隔条左侧那一栏：编辑模式与学生预览里都是它在 DOM 中的前一个兄弟。 */
function leftPaneOf(divider: HTMLElement): HTMLElement | null {
  return divider.previousElementSibling instanceof HTMLElement ? divider.previousElementSibling : null;
}

function containerOf(divider: HTMLElement): HTMLElement | null {
  return divider.closest<HTMLElement>(".workspace-body");
}

export function usePaneDivider(itemId: string): PaneDividerHandle {
  const [isDragging, setDragging] = useState(false);
  const dragState = useRef<DragState | null>(null);

  // 挂载时恢复该条目上次保存的宽度（读取失败就保持默认 clamp 值）。
  useEffect(() => {
    let saved: string | null = null;
    try {
      saved = window.localStorage.getItem(storageKey(itemId));
    } catch {
      return;
    }
    if (!saved) return;
    const px = Number.parseFloat(saved);
    if (!Number.isFinite(px)) return;
    const body = document.querySelector<HTMLElement>(".workspace-body");
    if (!body) return;
    body.style.setProperty("--workspace-left-pane-width", `${clampWidth(px, body)}px`);
  }, [itemId]);

  const applyWidth = useCallback((px: number, container: HTMLElement) => {
    container.style.setProperty("--workspace-left-pane-width", `${clampWidth(px, container)}px`);
  }, []);

  const currentWidth = useCallback((divider: HTMLElement): number => {
    const leftPane = leftPaneOf(divider);
    return leftPane ? leftPane.getBoundingClientRect().width : MIN_PANE_PX;
  }, []);

  const persist = useCallback((px: number) => {
    try {
      window.localStorage.setItem(storageKey(itemId), String(Math.round(px)));
    } catch {
      // localStorage 不可用（隐私模式等）：宽度只在本次会话内生效。
    }
  }, [itemId]);

  const onPointerDown = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    const divider = event.currentTarget;
    const container = containerOf(divider);
    if (!container) return; // 独立画布（legacy 路由）没有工作区网格：不可拖。
    event.preventDefault();
    dragState.current = { startX: event.clientX, startWidth: currentWidth(divider) };
    setDragging(true);
    document.body.classList.add("workspace-resizing");
    const onMove = (moveEvent: PointerEvent) => {
      const state = dragState.current;
      if (!state) return;
      applyWidth(state.startWidth + (moveEvent.clientX - state.startX), container);
    };
    const onUp = (upEvent: PointerEvent) => {
      const state = dragState.current;
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      document.body.classList.remove("workspace-resizing");
      dragState.current = null;
      setDragging(false);
      if (state) persist(state.startWidth + (upEvent.clientX - state.startX));
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  }, [applyWidth, currentWidth, persist]);

  const onKeyDown = useCallback((event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    const divider = event.currentTarget;
    const container = containerOf(divider);
    if (!container) return;
    event.preventDefault();
    const delta = event.key === "ArrowLeft" ? -KEYBOARD_STEP_PX : KEYBOARD_STEP_PX;
    const next = clampWidth(currentWidth(divider) + delta, container);
    applyWidth(next, container);
    persist(next);
  }, [applyWidth, currentWidth, persist]);

  const dividerProps: PaneDividerProps = {
    role: "separator",
    "aria-orientation": "vertical",
    "aria-label": "调整原文与题目宽度",
    tabIndex: 0,
    className: isDragging ? "workspace-divider is-dragging" : "workspace-divider",
    onPointerDown,
    onKeyDown
  };

  return { dividerProps, isDragging };
}
