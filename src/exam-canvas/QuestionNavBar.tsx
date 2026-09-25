import { useState } from "react";
import { findTargetElement } from "../features/editor/locate";
import type { IeltsAuthoringIRV2 } from "../types";
import { firstUnansweredSlot, type QuestionNavModel } from "./questionNavModel";

/**
 * 底部题号导航（基准页 .practice-nav 的一比一移植）：
 *   .practice-nav > .part-nav-section(.active) > .part-nav-info + .part-nav-questions > .q-item
 *
 * 由 ExamCanvas 在 author / student 两种模式下都渲染为画布的最后一个子元素；
 * 数据推导在 questionNavModel.ts（纯函数）。
 *
 * 两种模式语义一致：点击题号都滚动到对应题面。差异只有两处（允许的 mode 条件渲染）：
 *   - author 额外 onSelect 选中该答案位，.q-item.active 来自 props.selectedId；
 *   - student 的 active 记在本地 state（预览交互不写回题稿），且右侧多一个
 *     #submit-btn —— 按用户确认不做判分，点击只定位到第一道未作答的题。
 */
export function QuestionNavBar({
  model,
  mode,
  authoring,
  activeSlotId,
  onSelectSlot,
  onSelectPart
}: {
  model: QuestionNavModel;
  mode: "author" | "student";
  authoring: IeltsAuthoringIRV2;
  /** author 模式当前选中的 id；恰好是题位 id 时对应 .q-item 高亮。 */
  activeSlotId?: string;
  /** author 模式点击题号时选中答案位（透传 ExamCanvas 的 props.onSelect）。 */
  onSelectSlot?: (id: string) => void;
  /** 听力稿点击非当前 Part 时切换（与 ExamCanvas 的 selectedPart 同一 state）。 */
  onSelectPart: (ordinal: number) => void;
}) {
  const [localActive, setLocalActive] = useState<string>();
  const scrollToSlot = (slotId: string) => {
    findTargetElement(slotId, authoring)?.scrollIntoView({ block: "center", behavior: "smooth" });
  };
  const clickQuestion = (slotId: string) => {
    if (mode === "author") onSelectSlot?.(slotId);
    else setLocalActive(slotId);
    scrollToSlot(slotId);
  };
  const locateFirstUnanswered = () => {
    const slotId = firstUnansweredSlot(model);
    if (slotId) scrollToSlot(slotId);
  };

  return <nav className="practice-nav" data-testid="question-nav" aria-label="题目导航">
    {model.sections.map((section) => section.active ? (
      <div key={section.key} className="part-nav-section active">
        <div className="part-nav-info">
          <div className="part-nav-name">{section.name}</div>
          <div className="part-nav-status">{section.status}</div>
        </div>
        <div className="part-nav-questions">
          {section.questions.map((question) => {
            const active = mode === "author" ? activeSlotId === question.slotId : localActive === question.slotId;
            return <button
              key={question.slotId}
              type="button"
              className={`q-item${question.answered ? " answered" : ""}${active ? " active" : ""}`}
              onClick={() => clickQuestion(question.slotId)}
            >{question.displayNumber}</button>;
          })}
        </div>
      </div>
    ) : (
      // 非当前 Part：基准用 div + JS 点击，这里用真按钮保证键盘可达；
      // 题号块不渲染（基准 CSS 本来就把它 display:none，还能避免 button 嵌 button）。
      <div key={section.key} className="part-nav-section is-switchable">
        <button type="button" className="part-nav-info" onClick={() => onSelectPart(section.ordinal)}>
          <div className="part-nav-name">{section.name}</div>
          <div className="part-nav-status">{section.status}</div>
        </button>
      </div>
    ))}
    {mode === "student" ? <div className="nav-controls-right">
      <button
        type="button"
        id="submit-btn"
        title="定位到第一道未作答的题"
        aria-label="定位到第一道未作答的题"
        onClick={locateFirstUnanswered}
      >
        <svg className="submit-btn-icon" viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" strokeWidth={2.4} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
          <polyline points="20 6 9 17 4 12" />
        </svg>
      </button>
    </div> : null}
  </nav>;
}
