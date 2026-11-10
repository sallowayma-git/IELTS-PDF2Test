// 图表：SVG / CSS 自绘，不引入图表库。每个图的标题与 N 由调用方给出。
import type { HistBin } from "../../api/analyticsClient";
import { pct } from "./dataFormat";

export function Histogram({ bins, label }: { bins: HistBin[]; label: string }) {
  if (bins.length === 0) {
    return <p className="data-empty">暂无有效计分的学生。</p>;
  }
  const max = Math.max(1, ...bins.map((bin) => bin.count));
  const width = Math.max(480, bins.length * 24);
  const cell = width / bins.length;
  const height = 140;
  return (
    <div className="data-chart-scroll" role="region" aria-label={label} tabIndex={0}>
      <svg
        className="data-histogram"
        style={{ minWidth: Math.max(480, bins.length * 20) }}
        viewBox={`0 0 ${width} ${height}`}
        role="img"
        aria-label={label}
        data-testid="data-histogram"
        preserveAspectRatio="xMinYMid meet"
      >
        {bins.map((bin, index) => {
          const barHeight = (bin.count / max) * 100;
          const x = index * cell + 4;
          return (
            <g key={bin.from}>
              <rect x={x} y={110 - barHeight} width={cell - 8} height={barHeight} className="data-bar">
                <title>{`${bin.from}–${bin.to} 分：${bin.count} 人`}</title>
              </rect>
              {bin.count > 0 ? (
                <text x={x + (cell - 8) / 2} y={110 - barHeight - 3} textAnchor="middle" className="data-bar-count">
                  {bin.count}
                </text>
              ) : null}
              <text x={x + (cell - 8) / 2} y={126} textAnchor="middle" className="data-axis">
                {bin.from}
              </text>
            </g>
          );
        })}
        <line x1={0} y1={110} x2={width} y2={110} className="data-axis-line" />
      </svg>
    </div>
  );
}

export interface BarRowItem {
  key: string;
  label: string;
  /** 0–1；null 表示无数据。 */
  value: number | null;
  detail: string;
  onClick?: () => void;
}

/** 横向条形图：每行一个分类，宽度 = 正确率。 */
export function BarRows({ items, ariaLabel }: { items: BarRowItem[]; ariaLabel: string }) {
  if (items.length === 0) return <p className="data-empty">暂无数据。</p>;
  return (
    <ul className="data-bar-rows" aria-label={ariaLabel}>
      {items.map((item) => {
        const body = (
          <>
            <span className="data-bar-label">{item.label}</span>
            <span className="data-bar-track" aria-hidden="true">
              <span className="data-bar-fill" style={{ width: `${Math.round((item.value ?? 0) * 100)}%` }} />
            </span>
            <span className="data-bar-value">{pct(item.value)}</span>
            <span className="data-bar-detail">{item.detail}</span>
          </>
        );
        return (
          <li key={item.key}>
            {item.onClick ? (
              <button className="data-bar-row" type="button" onClick={item.onClick}>
                {body}
              </button>
            ) : (
              <div className="data-bar-row">{body}</div>
            )}
          </li>
        );
      })}
    </ul>
  );
}
