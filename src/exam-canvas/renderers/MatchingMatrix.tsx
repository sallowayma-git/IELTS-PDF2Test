import type { ReactNode } from "react";
import type { ContentNodeV2, OptionV2, ResponseGroupV2 } from "../../types";
import { matchingMatrixFor } from "./matchingMatrixModel";
import { optionContentDuplicatesLabel } from "../tfngOptions";

// Matching 矩阵（计划 §9.8）。
//
// 真实 IELTS 的 matching 版式是：左侧每题一行题干，上方一排 A/B/C 选项列，
// 选项正文在矩阵下方单独列出一次，而不是每题重复一遍完整选项文字。
// 数据上对应：同一个 TaskGroup 下多个 `kind: "matching"` 的 ResponseGroup，每个带一个 slot，
// 共享 task 级 optionBank 或相同的 inline 选项。
//
// 每行一个 slot，per_slot 用 radio 语义；只有明确多选（checkbox interaction）才用 checkbox。
export interface MatchingRow {
  responseGroup: ResponseGroupV2;
  slotId: string;
  displayNumber: string;
  prompt?: ContentNodeV2[];
  interaction: "radio" | "checkbox";
}

/** Same rule as NAS, including old compiler-generated paragraph options. */
export function matchingRowsFor(
  responseGroups: readonly ResponseGroupV2[],
  questionDisplayMap: Record<string, string>,
  interactionFor: (slotId: string) => string | undefined,
  optionsFor: (response: ResponseGroupV2) => OptionV2[],
  taskType: string,
  paragraphMap?: Record<string, string>
): MatchingRow[] {
  const matrix = matchingMatrixFor({ taskType, responseGroups: [...responseGroups] }, optionsFor, interactionFor, paragraphMap);
  return matrix?.rows.map(({ response: responseGroup, slotId, interaction }) => ({
      responseGroup,
      slotId,
      displayNumber: questionDisplayMap[slotId] ?? "",
      prompt: responseGroup.prompt,
      interaction
    })) ?? [];
}

export function MatchingMatrix({
  rows,
  options,
  answers,
  selectedId,
  interactive,
  renderContent,
  onSelectOption,
  onSelectTarget
}: {
  rows: MatchingRow[];
  options: OptionV2[];
  answers: Record<string, string[]>;
  selectedId?: string;
  /** 作者模式与学生模式都允许勾选：作者勾选就是设置答案。 */
  interactive: boolean;
  renderContent: (nodes: ContentNodeV2[] | undefined) => ReactNode;
  onSelectOption: (slotId: string, label: string, checked: boolean, multiple: boolean) => void;
  onSelectTarget?: (id: string) => void;
}) {
  if (!rows.length || !options.length) return null;

  return (
    <div className="v2-matching-matrix" data-testid="matching-matrix">
      <table>
        <thead>
          <tr>
            <th scope="col" className="v2-matching-item-head">Question</th>
            {options.map((option) => (
              <th scope="col" key={option.optionId} className="v2-matching-option-head">{option.label}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => {
            const values = answers[row.slotId] ?? [];
            return (
              <tr
                key={row.slotId}
                className={selectedId === row.responseGroup.responseGroupId ? "is-selected" : ""}
                data-question-id={row.slotId}
                onClick={(event) => {
                  if (!onSelectTarget) return;
                  event.stopPropagation();
                  onSelectTarget(row.responseGroup.responseGroupId);
                }}
              >
                <th scope="row" className="v2-matching-item">
                  <span className="v2-slot-number">{row.displayNumber}</span>
                  <span className="v2-matching-prompt">{renderContent(row.prompt)}</span>
                </th>
                {options.map((option) => (
                  <td key={`${row.slotId}-${option.optionId}`} className="v2-matching-cell">
                    <label>
                      <input
                        type={row.interaction}
                        name={row.slotId}
                        value={option.label}
                        checked={values.includes(option.label)}
                        disabled={!interactive}
                        aria-label={`${row.displayNumber} Choose ${option.label}`}
                        onChange={(event) =>
                          onSelectOption(row.slotId, option.label, event.target.checked, row.interaction === "checkbox")
                        }
                      />
                    </label>
                  </td>
                ))}
              </tr>
            );
          })}
        </tbody>
      </table>

      {options.some(option => !optionContentDuplicatesLabel(option)) ? <ol className="v2-matching-legend">
        {options.map((option) => (
          <li key={option.optionId}>
            <strong>{option.label}</strong>
            <span>{renderContent(option.content)}</span>
          </li>
        ))}
      </ol> : null}
    </div>
  );
}
